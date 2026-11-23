// SPDX-License-Identifier: AGPL-3.0-or-later
//! Order lifecycle: create (server-priced, modifiers, stock), confirm/reject, ready, discount (with approval),
//! item void, table move, pay (locks), void and refund (supervisor approval, always audited).

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, Perm, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, load_settings, rules},
    money::{self, rupiah},
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::SqliteConnection;

#[derive(Debug, Clone)]
pub struct LineReq {
    pub menu_id: String,
    pub qty: i64,
    pub mods: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct PriceRow {
    id: String,
    name: String,
    price: i64,
    stock: Option<i64>,
}

pub struct NewOrder<'a> {
    pub source: &'a str,
    pub table_id: Option<&'a str>,
    pub cashier_id: Option<&'a str>,
    pub items: Vec<LineReq>,
    pub note: Option<String>,
}

/// Prices, modifiers and stock all come from the database, never from the client.
pub async fn create_order(conn: &mut SqliteConnection, st: &AppState, o: NewOrder<'_>) -> AppResult<(String, i64)> {
    let settings = load_settings(conn).await?;
    let mut lines: Vec<(String, String, i64, i64)> = Vec::new(); // menu_id, name, unit_price, qty
    let mut subtotal = 0i64;
    for req in &o.items {
        let row: Option<PriceRow> = sqlx::query_as(
            "SELECT m.id, m.name, m.price, m.stock FROM menu_items m JOIN categories c ON c.id = m.category_id \
             WHERE m.id = ? AND m.active = 1 AND m.available = 1 AND c.active = 1",
        )
        .bind(&req.menu_id)
        .fetch_optional(&mut *conn)
        .await?;
        let r = row
            .ok_or_else(|| AppError::Invalid("Ada menu yang habis atau tidak tersedia. Muat ulang halaman.".into()))?;
        if let Some(stock) = r.stock {
            if stock < req.qty {
                return Err(AppError::Invalid(format!("Stok {} tinggal {}.", r.name, stock)));
            }
            sqlx::query("UPDATE menu_items SET stock = stock - ?, available = CASE WHEN stock - ? = 0 THEN 0 ELSE available END WHERE id = ?")
                .bind(req.qty)
                .bind(req.qty)
                .bind(&r.id)
                .execute(&mut *conn)
                .await?;
        }
        let (extra, label) = resolve_modifiers(conn, &r.id, &req.mods).await?;
        let name = if label.is_empty() {
            r.name.clone()
        } else {
            format!("{} ({})", r.name, label)
        };
        let unit = r.price + extra;
        subtotal += unit * req.qty;
        lines.push((r.id, name, unit, req.qty));
    }
    let t = money::compute(subtotal, 0, rules(&settings), settings.round_to);
    let now = clock::now();
    let day = clock::day(now, st.cfg.tz_offset_hours);
    // Atomic per-day counter: no duplicate or skipped numbers.
    let number: i64 = sqlx::query_scalar(
        "INSERT INTO daily_counters (day, n) VALUES (?, 1) ON CONFLICT(day) DO UPDATE SET n = n + 1 RETURNING n",
    )
    .bind(&day)
    .fetch_one(&mut *conn)
    .await?;
    let id = rand_id::id();
    let status = if o.source == "qr" { "pending" } else { "confirmed" };
    sqlx::query(
        "INSERT INTO orders (id, day, number, table_id, source, status, subtotal, discount, service, tax, rounding, total, note, cashier_id, created_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&day)
    .bind(number)
    .bind(o.table_id)
    .bind(o.source)
    .bind(status)
    .bind(t.subtotal)
    .bind(t.discount)
    .bind(t.service)
    .bind(t.tax)
    .bind(t.rounding)
    .bind(t.total)
    .bind(&o.note)
    .bind(o.cashier_id)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    for (menu_id, name, unit, qty) in lines {
        // Name (with modifiers) and price are copied: later menu edits never rewrite history.
        sqlx::query("INSERT INTO order_items (id, order_id, menu_id, name, unit_price, qty) VALUES (?,?,?,?,?,?)")
            .bind(rand_id::id())
            .bind(&id)
            .bind(&menu_id)
            .bind(&name)
            .bind(unit)
            .bind(qty)
            .execute(&mut *conn)
            .await?;
    }
    audit::record(conn, Entry::new(o.cashier_id, "order.created", "order", Some(&id))).await?;
    Ok((id, number))
}

/// Validate the chosen modifiers against the item's groups. Returns (extra price, "A, B" label).
async fn resolve_modifiers(conn: &mut SqliteConnection, menu_id: &str, chosen: &[String]) -> AppResult<(i64, String)> {
    let groups: Vec<(String, String, i64, i64)> =
        sqlx::query_as("SELECT id, name, required, max_select FROM modifier_groups WHERE menu_id = ?")
            .bind(menu_id)
            .fetch_all(&mut *conn)
            .await?;
    let mods: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT m.id, m.group_id, m.name, m.price FROM modifiers m JOIN modifier_groups g ON g.id = m.group_id WHERE g.menu_id = ? AND m.active = 1",
    )
    .bind(menu_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut seen = std::collections::HashSet::new();
    let mut picked = Vec::new();
    for c in chosen {
        if !seen.insert(c.clone()) {
            continue;
        }
        let m = mods
            .iter()
            .find(|m| &m.0 == c)
            .ok_or_else(|| AppError::Invalid("Pilihan varian tidak valid.".into()))?;
        picked.push(m);
    }
    for (gid, gname, required, max) in &groups {
        let n = picked.iter().filter(|m| &m.1 == gid).count() as i64;
        if *required == 1 && n == 0 {
            return Err(AppError::Invalid(format!("Pilih {gname} terlebih dahulu.")));
        }
        if n > *max {
            return Err(AppError::Invalid(format!("{gname}: maksimal {max} pilihan.")));
        }
    }
    let extra = picked.iter().map(|m| m.3).sum();
    let label = picked.iter().map(|m| m.2.as_str()).collect::<Vec<_>>().join(", ");
    Ok((extra, label))
}

/// Give stock back when an unpaid order or one of its items is cancelled.
pub async fn restore_stock(conn: &mut SqliteConnection, order_id: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE menu_items SET stock = stock + (SELECT COALESCE(SUM(i.qty),0) FROM order_items i WHERE i.order_id = ?1 AND i.menu_id = menu_items.id AND i.voided = 0), \
         available = CASE WHEN stock = 0 THEN 1 ELSE available END \
         WHERE stock IS NOT NULL AND id IN (SELECT menu_id FROM order_items WHERE order_id = ?1 AND voided = 0)",
    )
    .bind(order_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Recompute totals from live (non-voided) items, keeping the discount (clamped).
async fn recalc(conn: &mut SqliteConnection, id: &str, discount: Option<i64>) -> AppResult<money::Totals> {
    let settings = load_settings(conn).await?;
    let subtotal: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(unit_price * qty),0) FROM order_items WHERE order_id = ? AND voided = 0",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let current: i64 = sqlx::query_scalar("SELECT discount FROM orders WHERE id = ?")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let t = money::compute(
        subtotal,
        discount.unwrap_or(current),
        rules(&settings),
        settings.round_to,
    );
    sqlx::query("UPDATE orders SET subtotal=?, discount=?, service=?, tax=?, rounding=?, total=? WHERE id=?")
        .bind(t.subtotal)
        .bind(t.discount)
        .bind(t.service)
        .bind(t.tax)
        .bind(t.rounding)
        .bind(t.total)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(t)
}

pub fn notify(st: &AppState) {
    let _ = st.events.send("orders".to_string());
}

#[derive(sqlx::FromRow)]
struct OrderFull {
    id: String,
    number: i64,
    status: String,
    subtotal: i64,
    discount: i64,
    service: i64,
    tax: i64,
    rounding: i64,
    total: i64,
    table_label: Option<String>,
}

async fn fetch_order(conn: &mut SqliteConnection, id: &str) -> AppResult<OrderFull> {
    sqlx::query_as(
        "SELECT o.id, o.number, o.status, o.subtotal, o.discount, o.service, o.tax, o.rounding, o.total, t.label AS table_label \
         FROM orders o LEFT JOIN tables t ON t.id = o.table_id WHERE o.id = ?",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or(AppError::NotFound)
}

pub fn status_meta(s: &str) -> (&'static str, &'static str) {
    match s {
        "pending" => ("Menunggu konfirmasi", "warn"),
        "confirmed" => ("Diproses", "neutral"),
        "ready" => ("Siap", "ok"),
        "paid" => ("Lunas", "ok"),
        "voided" => ("Void", "danger"),
        _ => ("Dibatalkan", "danger"),
    }
}
pub use status_meta as order_status_meta;

/// Next round banknote amounts above the total (50k, 100k, ...) for one-tap tendering.
fn quick_cash(total: i64) -> Vec<i64> {
    let mut v: Vec<i64> = [20_000, 50_000, 100_000, 200_000]
        .into_iter()
        .map(|b| (total + b - 1) / b * b)
        .filter(|x| *x > total)
        .collect();
    v.sort();
    v.dedup();
    v.truncate(3);
    v
}

fn pct_text(bp: i64) -> String {
    if bp % 100 == 0 {
        (bp / 100).to_string()
    } else {
        format!("{}.{:02}", bp / 100, bp % 100)
    }
}

pub async fn render_detail(
    st: &AppState,
    staff: &Staff,
    id: &str,
    error: Option<String>,
    notice: Option<String>,
) -> AppResult<Response> {
    let mut c = st.db.acquire().await?;
    let o = fetch_order(&mut c, id).await?;
    let settings = load_settings(&mut c).await?;
    let lines: Vec<(String, i64, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, qty, name, unit_price * qty, voided, void_reason FROM order_items WHERE order_id = ?",
    )
    .bind(id)
    .fetch_all(&mut *c)
    .await?;
    let shift_open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM shifts WHERE closed_at IS NULL)")
        .fetch_one(&mut *c)
        .await?;
    let tables: Vec<TableRow> = sqlx::query_as("SELECT id, label, active FROM tables WHERE active = 1 ORDER BY label")
        .fetch_all(&mut *c)
        .await?;
    let refunds: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT amount, reason, created_at FROM refunds WHERE order_id = ? ORDER BY created_at")
            .bind(id)
            .fetch_all(&mut *c)
            .await?;
    let refunded: i64 = refunds.iter().map(|r| r.0).sum();
    let tz = st.cfg.tz_offset_hours;
    let (label, class) = status_meta(&o.status);
    let editable = matches!(o.status.as_str(), "pending" | "confirmed" | "ready");
    let page = OrderDetail {
        ctx: Ctx::staff(st, staff, "cashier").await?,
        error,
        notice,
        where_label: o
            .table_label
            .clone()
            .map(|l| format!("Meja {l}"))
            .unwrap_or_else(|| "Antre".into()),
        lines: lines
            .into_iter()
            .map(|(id, qty, name, l, v, r)| ItemLine {
                id,
                qty,
                name,
                line_fmt: rupiah(l),
                voided: v == 1,
                void_reason: r.unwrap_or_default(),
            })
            .collect(),
        subtotal: rupiah(o.subtotal),
        discount: rupiah(o.discount),
        has_discount: o.discount > 0,
        service: rupiah(o.service),
        tax: rupiah(o.tax),
        rounding: rupiah(o.rounding),
        has_rounding: o.rounding != 0,
        total: rupiah(o.total),
        quick: quick_cash(o.total).into_iter().map(|v| (v, rupiah(v))).collect(),
        status_label: label,
        status_class: class,
        shift_open,
        editable,
        can_discount: staff.can(Perm::Discount) && editable,
        can_refund: staff.can(Perm::Refund) && o.status == "paid",
        discount_limit_pct: pct_text(settings.cashier_discount_max_bp),
        tables,
        refunded: rupiah(refunded),
        refund_left: o.total - refunded,
        refunds: refunds
            .into_iter()
            .map(|(a, r, at)| (format!("{} {}", clock::day(at, tz), clock::hm(at, tz)), rupiah(a), r))
            .collect(),
        o: OrderRow {
            id: o.id,
            number: o.number,
            status: o.status,
            total: o.total,
        },
    };
    Ok(render(page)?.into_response())
}

pub async fn detail(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    render_detail(&st, &staff, &id, None, None).await
}

pub async fn confirm(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let n = sqlx::query("UPDATE orders SET status = 'confirmed', cashier_id = ? WHERE id = ? AND status = 'pending'")
        .bind(&staff.id)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::Invalid("Pesanan sudah diproses atau tidak ditemukan.".into()));
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "order.confirmed", "order", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    notify(&st);
    super::receipt::print_ticket(st.clone(), id);
    Ok(Redirect::to("/cashier"))
}

#[derive(Deserialize)]
pub struct ReasonForm {
    reason: String,
}

pub async fn reject(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<ReasonForm>,
) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let reason = clean(&f.reason, 120);
    if reason.is_empty() {
        return Err(AppError::Invalid("Alasan wajib diisi.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let n =
        sqlx::query("UPDATE orders SET status = 'cancelled', cancel_reason = ? WHERE id = ? AND status = 'pending'")
            .bind(&reason)
            .bind(&id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if n == 0 {
        return Err(AppError::Invalid("Pesanan sudah diproses atau tidak ditemukan.".into()));
    }
    restore_stock(&mut tx, &id).await?;
    let mut e = Entry::new(Some(&staff.id), "order.rejected", "order", Some(&id));
    e.reason = Some(&reason);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    notify(&st);
    Ok(Redirect::to("/cashier"))
}

pub async fn ready(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Kitchen, Role::Cashier])?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE orders SET status = 'ready' WHERE id = ? AND status = 'confirmed'")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(&mut tx, Entry::new(Some(&staff.id), "order.ready", "order", Some(&id))).await?;
    tx.commit().await?;
    notify(&st);
    Ok(Redirect::to(staff.role.home()))
}

// ---- Approval -------------------------------------------------------------------------------

/// A second person (owner) authorises a sensitive action with their own credentials.
/// An owner acting alone may re-enter their own passphrase (still audited as approver).
pub async fn approve(st: &AppState, actor: &Staff, name: &str, secret: &str) -> Result<Staff, String> {
    match auth::check_credentials(&st.db, name, secret).await {
        Ok(a) if a.role == Role::Owner && (a.id != actor.id || actor.role == Role::Owner) => Ok(a),
        Ok(_) => Err("Penyetuju harus owner (bukan kasir yang sama).".into()),
        Err(_) => Err("Kredensial penyetuju salah atau terkunci.".into()),
    }
}

// ---- Discount -------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct DiscountForm {
    mode: String,
    value: i64,
    reason: String,
    approver: Option<String>,
    secret: Option<String>,
}

pub async fn discount(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<DiscountForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    staff.need(Perm::Discount)?;
    let reason = clean(&f.reason, 120);
    if reason.is_empty() || f.value < 0 {
        return render_detail(
            &st,
            &staff,
            &id,
            Some("Alasan wajib dan nilai tidak boleh negatif.".into()),
            None,
        )
        .await;
    }
    let mut c = st.db.acquire().await?;
    let settings = load_settings(&mut c).await?;
    let o = fetch_order(&mut c, &id).await?;
    drop(c);
    if !matches!(o.status.as_str(), "pending" | "confirmed" | "ready") {
        return Err(AppError::Invalid("Diskon hanya bisa diberikan sebelum dibayar.".into()));
    }
    let amount = match f.mode.as_str() {
        "pct" if f.value <= 100 => o.subtotal * f.value / 100,
        "rp" => f.value,
        _ => return render_detail(&st, &staff, &id, Some("Diskon tidak valid.".into()), None).await,
    }
    .min(o.subtotal);
    // Over the cashier's limit -> an owner must approve with their own credentials.
    let over = amount * 10_000 > o.subtotal * settings.cashier_discount_max_bp;
    let mut approver_id = None;
    if over && staff.role != Role::Owner {
        match approve(
            &st,
            &staff,
            f.approver.as_deref().unwrap_or(""),
            f.secret.as_deref().unwrap_or(""),
        )
        .await
        {
            Ok(a) => approver_id = Some(a.id),
            Err(e) => {
                return render_detail(&st, &staff, &id, Some(format!("Diskon di atas batas kasir: {e}")), None).await;
            }
        }
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    recalc(&mut tx, &id, Some(amount)).await?;
    let detail = format!("{} {} => {}", f.mode, f.value, amount);
    let mut e = Entry::new(Some(&staff.id), "order.discount", "order", Some(&id));
    e.approver = approver_id.as_deref();
    e.reason = Some(&reason);
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    render_detail(&st, &staff, &id, None, Some("Diskon diterapkan.".into())).await
}

// ---- Item void (before payment) --------------------------------------------------------------

pub async fn void_item(
    State(st): State<AppState>,
    staff: Staff,
    Path((id, item)): Path<(String, String)>,
    Form(f): Form<ReasonForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let reason = clean(&f.reason, 120);
    if reason.is_empty() {
        return render_detail(&st, &staff, &id, Some("Alasan wajib diisi.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let o = fetch_order(&mut tx, &id).await?;
    if !matches!(o.status.as_str(), "pending" | "confirmed" | "ready") {
        return Err(AppError::Invalid("Item hanya bisa dibatalkan sebelum dibayar.".into()));
    }
    let row: Option<(String, i64, i64)> =
        sqlx::query_as("SELECT menu_id, qty, voided FROM order_items WHERE id = ? AND order_id = ?")
            .bind(&item)
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let (menu_id, qty, voided) = row.ok_or(AppError::NotFound)?;
    if voided == 1 {
        return Err(AppError::Invalid("Item sudah dibatalkan.".into()));
    }
    sqlx::query("UPDATE order_items SET voided = 1, void_reason = ? WHERE id = ?")
        .bind(&reason)
        .bind(&item)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE menu_items SET stock = stock + ?, available = CASE WHEN stock = 0 THEN 1 ELSE available END WHERE id = ? AND stock IS NOT NULL")
        .bind(qty)
        .bind(&menu_id)
        .execute(&mut *tx)
        .await?;
    let t = recalc(&mut tx, &id, None).await?;
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_items WHERE order_id = ? AND voided = 0")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    if left == 0 {
        sqlx::query("UPDATE orders SET status = 'cancelled', cancel_reason = ? WHERE id = ?")
            .bind("Semua item dibatalkan")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    let mut e = Entry::new(Some(&staff.id), "order.item_voided", "order_item", Some(&item));
    e.reason = Some(&reason);
    let detail = format!("order {id} new total {}", t.total);
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    notify(&st);
    render_detail(&st, &staff, &id, None, Some("Item dibatalkan.".into())).await
}

#[derive(Deserialize)]
pub struct MoveForm {
    table_id: String,
}

pub async fn move_table(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<MoveForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let o = fetch_order(&mut tx, &id).await?;
    if !matches!(o.status.as_str(), "pending" | "confirmed" | "ready") {
        return Err(AppError::Invalid("Meja hanya bisa dipindah sebelum dibayar.".into()));
    }
    let target: Option<String> = if f.table_id.is_empty() {
        None
    } else {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tables WHERE id = ? AND active = 1)")
            .bind(&f.table_id)
            .fetch_one(&mut *tx)
            .await?;
        if !ok {
            return render_detail(&st, &staff, &id, Some("Meja tidak valid.".into()), None).await;
        }
        Some(f.table_id.clone())
    };
    sqlx::query("UPDATE orders SET table_id = ? WHERE id = ?")
        .bind(&target)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "order.table_moved", "order", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    notify(&st);
    render_detail(&st, &staff, &id, None, Some("Meja dipindahkan.".into())).await
}

// ---- Payment ---------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PayForm {
    method: String,
    tendered: Option<String>,
    reference: Option<String>,
}

pub async fn pay(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<PayForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    if !["cash", "qris", "card", "transfer"].contains(&f.method.as_str()) {
        return Err(AppError::Invalid("Metode bayar tidak dikenal.".into()));
    }
    let reference = clean(f.reference.as_deref().unwrap_or(""), 60);
    // Never store a full card number: a reference that looks like a PAN (13+ digits) is refused.
    if reference.chars().filter(|c| c.is_ascii_digit()).count() >= 13 && f.method == "card" {
        return Err(AppError::Invalid(
            "Jangan masukkan nomor kartu. Isi nomor referensi/approval EDC saja.".into(),
        ));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let o = fetch_order(&mut tx, &id).await?;
    if o.status != "confirmed" && o.status != "ready" {
        return Err(AppError::Invalid("Pesanan ini tidak bisa dibayar.".into()));
    }
    let shift: Option<String> = sqlx::query_scalar("SELECT id FROM shifts WHERE closed_at IS NULL")
        .fetch_optional(&mut *tx)
        .await?;
    let shift = shift.ok_or_else(|| AppError::Invalid("Buka shift sebelum menerima pembayaran.".into()))?;
    let tendered: Option<i64> = if f.method == "cash" {
        let t: i64 = f
            .tendered
            .as_deref()
            .unwrap_or("")
            .trim()
            .parse()
            .map_err(|_| AppError::Invalid("Uang diterima tidak valid.".into()))?;
        if t < o.total {
            drop(tx);
            return render_detail(&st, &staff, &id, Some("Uang diterima kurang dari total.".into()), None).await;
        }
        Some(t)
    } else {
        if reference.is_empty() {
            drop(tx);
            return render_detail(
                &st,
                &staff,
                &id,
                Some("Nomor referensi wajib untuk pembayaran non-tunai.".into()),
                None,
            )
            .await;
        }
        None
    };
    let now = clock::now();
    sqlx::query("INSERT INTO payments (id, order_id, shift_id, method, amount, tendered, reference, cashier_id, created_at) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(rand_id::id())
        .bind(&id)
        .bind(&shift)
        .bind(&f.method)
        .bind(o.total)
        .bind(tendered)
        .bind(if reference.is_empty() { None } else { Some(&reference) })
        .bind(&staff.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE orders SET status = 'paid', paid_at = ?, cashier_id = ? WHERE id = ?")
        .bind(now)
        .bind(&staff.id)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let mut e = Entry::new(Some(&staff.id), "order.paid", "order", Some(&id));
    let detail = format!("{} {}", f.method, o.total);
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    notify(&st);
    super::receipt::auto_print(st.clone(), id.clone());
    Ok(Redirect::to(&format!("/orders/{id}/receipt")).into_response())
}

#[derive(Deserialize)]
pub struct VoidForm {
    reason: String,
    approver: String,
    secret: String,
}

/// Void after payment: needs a reason AND an owner approval (see `approve`).
pub async fn void(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<VoidForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let reason = clean(&f.reason, 160);
    if reason.is_empty() {
        return Err(AppError::Invalid("Alasan wajib diisi.".into()));
    }
    let approver = match approve(&st, &staff, &f.approver, &f.secret).await {
        Ok(a) => a,
        Err(e) => return render_detail(&st, &staff, &id, Some(e), None).await,
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    let n = sqlx::query("UPDATE orders SET status = 'voided', cancel_reason = ? WHERE id = ? AND status = 'paid'")
        .bind(&reason)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::Invalid("Hanya pesanan lunas yang bisa di-void.".into()));
    }
    let mut e = Entry::new(Some(&staff.id), "order.voided", "order", Some(&id));
    e.approver = Some(&approver.id);
    e.reason = Some(&reason);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/orders/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct RefundForm {
    amount: i64,
    reason: String,
    approver: String,
    secret: String,
}

/// Partial or full refund as its own immutable document (FR-16). Cash refunds leave the drawer: recorded as cash-out.
pub async fn refund(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<RefundForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    staff.need(Perm::Refund)?;
    let reason = clean(&f.reason, 160);
    if reason.is_empty() || f.amount <= 0 {
        return render_detail(&st, &staff, &id, Some("Nominal dan alasan wajib diisi.".into()), None).await;
    }
    let approver = match approve(&st, &staff, &f.approver, &f.secret).await {
        Ok(a) => a,
        Err(e) => return render_detail(&st, &staff, &id, Some(e), None).await,
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    let o = fetch_order(&mut tx, &id).await?;
    if o.status != "paid" {
        return Err(AppError::Invalid("Refund hanya untuk pesanan lunas.".into()));
    }
    let done: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM refunds WHERE order_id = ?")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    if f.amount > o.total - done {
        drop(tx);
        return render_detail(
            &st,
            &staff,
            &id,
            Some("Nominal melebihi sisa yang bisa di-refund.".into()),
            None,
        )
        .await;
    }
    let method: String = sqlx::query_scalar("SELECT method FROM payments WHERE order_id = ?")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    let now = clock::now();
    let rid = rand_id::id();
    sqlx::query(
        "INSERT INTO refunds (id, order_id, amount, reason, actor_id, approver_id, created_at) VALUES (?,?,?,?,?,?,?)",
    )
    .bind(&rid)
    .bind(&id)
    .bind(f.amount)
    .bind(&reason)
    .bind(&staff.id)
    .bind(&approver.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if method == "cash" {
        let shift: Option<String> = sqlx::query_scalar("SELECT id FROM shifts WHERE closed_at IS NULL")
            .fetch_optional(&mut *tx)
            .await?;
        let Some(shift) = shift else {
            drop(tx);
            return render_detail(
                &st,
                &staff,
                &id,
                Some("Buka shift dulu: refund tunai mengurangi uang di laci.".into()),
                None,
            )
            .await;
        };
        let why = format!("Refund #{}", o.number);
        sqlx::query("INSERT INTO cash_movements (id, shift_id, kind, amount, reason, user_id, created_at) VALUES (?,?,?,?,?,?,?)")
            .bind(rand_id::id())
            .bind(&shift)
            .bind("out")
            .bind(f.amount)
            .bind(&why)
            .bind(&staff.id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    let mut e = Entry::new(Some(&staff.id), "order.refunded", "order", Some(&id));
    e.approver = Some(&approver.id);
    e.reason = Some(&reason);
    let detail = format!("amount {}", f.amount);
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    render_detail(&st, &staff, &id, None, Some("Refund dicatat.".into())).await
}

/// Shared by cashier queue and kitchen tickets.
pub async fn queue_orders(st: &AppState, statuses: &[&str]) -> AppResult<Vec<QueueOrder>> {
    // Statuses are passed as a JSON array: one fixed, fully parameterised statement.
    let statuses_json = serde_json::to_string(statuses)?;
    let q = sqlx::query_as::<_, (String, i64, String, String, i64, Option<String>, i64, Option<String>)>(
        "SELECT o.id, o.number, o.status, o.source, o.total, o.note, o.created_at, t.label \
         FROM orders o LEFT JOIN tables t ON t.id = o.table_id \
         WHERE o.status IN (SELECT value FROM json_each(?)) ORDER BY o.created_at",
    )
    .bind(statuses_json);
    let rows = q.fetch_all(&st.db).await?;
    let mut out = Vec::new();
    for (id, number, status, source, total, note, at, label) in rows {
        let lines: Vec<(i64, String)> =
            sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ? AND voided = 0")
                .bind(&id)
                .fetch_all(&st.db)
                .await?;
        out.push(QueueOrder {
            id,
            number,
            status,
            where_label: label.map(|l| format!("Meja {l}")).unwrap_or_else(|| "Antre".into()),
            source_label: if source == "qr" {
                "Pesanan QR".into()
            } else {
                "Kasir".into()
            },
            time: clock::hm(at, st.cfg.tz_offset_hours),
            lines: lines.into_iter().map(|(q, n)| format!("{q}× {n}")).collect(),
            note,
            total_fmt: rupiah(total),
        });
    }
    Ok(out)
}

/// Collect `qty_<menu id>` and `mod_<menu id>` fields into order lines. Bounded to stop abuse.
pub fn parse_lines(fields: &[(String, String)], max_qty: i64) -> Result<Vec<LineReq>, AppError> {
    let mut out: Vec<LineReq> = Vec::new();
    for (k, v) in fields {
        if let Some(id) = k.strip_prefix("qty_") {
            let q: i64 = v
                .trim()
                .parse()
                .map_err(|_| AppError::Invalid("Jumlah tidak valid.".into()))?;
            if !(0..=max_qty).contains(&q) {
                return Err(AppError::Invalid("Jumlah di luar batas.".into()));
            }
            if q > 0 {
                if id.len() > 64 || out.len() >= 40 {
                    return Err(AppError::Invalid("Terlalu banyak item.".into()));
                }
                // Field name is `mod_<menu id>__<group id>`; empty values mean "none".
                let mods: Vec<String> = fields
                    .iter()
                    .filter(|(mk, mv)| {
                        !mv.trim().is_empty()
                            && mk.strip_prefix("mod_").and_then(|r| r.split_once("__")).map(|(m, _)| m) == Some(id)
                    })
                    .map(|(_, mv)| mv.trim().to_string())
                    .take(20)
                    .collect();
                out.push(LineReq {
                    menu_id: id.to_string(),
                    qty: q,
                    mods,
                });
            }
        }
    }
    if out.is_empty() {
        return Err(AppError::Invalid("Pilih minimal satu menu.".into()));
    }
    Ok(out)
}

pub async fn parse_manual(fields: Vec<(String, String)>) -> AppResult<(Vec<LineReq>, Option<String>, Option<String>)> {
    let note = fields
        .iter()
        .find(|(k, _)| k == "note")
        .map(|(_, v)| clean(v, 200))
        .filter(|v| !v.is_empty());
    let table = fields
        .iter()
        .find(|(k, _)| k == "table_id")
        .map(|(_, v)| clean(v, 64))
        .filter(|v| !v.is_empty());
    Ok((parse_lines(&fields, 99)?, note, table))
}
