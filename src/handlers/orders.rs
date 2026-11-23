// SPDX-License-Identifier: AGPL-3.0-or-later
//! Order lifecycle: create (server-priced), confirm/reject, ready, pay (locks), void (supervisor approval).

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, load_settings, parse_qty, rules},
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

#[derive(sqlx::FromRow)]
struct PriceRow {
    id: String,
    name: String,
    price: i64,
}

pub struct NewOrder<'a> {
    pub source: &'a str,
    pub table_id: Option<&'a str>,
    pub cashier_id: Option<&'a str>,
    pub items: Vec<(String, i64)>,
    pub note: Option<String>,
}

/// Prices come from the database, never from the client. Returns (order id, number).
pub async fn create_order(conn: &mut SqliteConnection, st: &AppState, o: NewOrder<'_>) -> AppResult<(String, i64)> {
    let settings = load_settings(conn).await?;
    let mut lines = Vec::new();
    let mut subtotal = 0i64;
    for (menu_id, qty) in &o.items {
        let row: Option<PriceRow> = sqlx::query_as(
            "SELECT m.id, m.name, m.price FROM menu_items m JOIN categories c ON c.id = m.category_id \
             WHERE m.id = ? AND m.active = 1 AND m.available = 1 AND c.active = 1",
        )
        .bind(menu_id)
        .fetch_optional(&mut *conn)
        .await?;
        let r = row
            .ok_or_else(|| AppError::Invalid("Ada menu yang habis atau tidak tersedia. Muat ulang halaman.".into()))?;
        subtotal += r.price * qty;
        lines.push((r, *qty));
    }
    let t = money::compute(subtotal, rules(&settings));
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
        "INSERT INTO orders (id, day, number, table_id, source, status, subtotal, service, tax, total, note, cashier_id, created_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id).bind(&day).bind(number).bind(o.table_id).bind(o.source).bind(status)
    .bind(t.subtotal).bind(t.service).bind(t.tax).bind(t.total)
    .bind(&o.note).bind(o.cashier_id).bind(now)
    .execute(&mut *conn).await?;
    for (r, qty) in lines {
        // Name and price are copied: later menu edits never rewrite history.
        sqlx::query("INSERT INTO order_items (id, order_id, menu_id, name, unit_price, qty) VALUES (?,?,?,?,?,?)")
            .bind(rand_id::id())
            .bind(&id)
            .bind(&r.id)
            .bind(&r.name)
            .bind(r.price)
            .bind(qty)
            .execute(&mut *conn)
            .await?;
    }
    audit::record(conn, Entry::new(o.cashier_id, "order.created", "order", Some(&id))).await?;
    Ok((id, number))
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
    service: i64,
    tax: i64,
    total: i64,
    table_label: Option<String>,
}

async fn fetch_order(conn: &mut SqliteConnection, id: &str) -> AppResult<OrderFull> {
    sqlx::query_as(
        "SELECT o.id, o.number, o.status, o.subtotal, o.service, o.tax, o.total, t.label AS table_label \
         FROM orders o LEFT JOIN tables t ON t.id = o.table_id WHERE o.id = ?",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or(AppError::NotFound)
}

fn status_meta(s: &str) -> (&'static str, &'static str) {
    match s {
        "pending" => ("Menunggu konfirmasi", "warn"),
        "confirmed" => ("Diproses", "neutral"),
        "ready" => ("Siap", "ok"),
        "paid" => ("Lunas", "ok"),
        "voided" => ("Void", "danger"),
        _ => ("Dibatalkan", "danger"),
    }
}

async fn render_detail(st: &AppState, staff: &Staff, id: &str, error: Option<String>) -> AppResult<Response> {
    let mut c = st.db.acquire().await?;
    let o = fetch_order(&mut c, id).await?;
    let lines: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT qty, name, unit_price * qty FROM order_items WHERE order_id = ?")
            .bind(id)
            .fetch_all(&mut *c)
            .await?;
    let shift_open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM shifts WHERE closed_at IS NULL)")
        .fetch_one(&mut *c)
        .await?;
    let (label, class) = status_meta(&o.status);
    let page = OrderDetail {
        ctx: Ctx::staff(st, staff, "cashier").await?,
        error,
        where_label: o
            .table_label
            .map(|l| format!("Meja {l}"))
            .unwrap_or_else(|| "Antre".into()),
        lines: lines
            .into_iter()
            .map(|(qty, name, l)| LineView {
                qty,
                name,
                line_fmt: rupiah(l),
            })
            .collect(),
        subtotal: rupiah(o.subtotal),
        service: rupiah(o.service),
        tax: rupiah(o.tax),
        total: rupiah(o.total),
        status_label: label,
        status_class: class,
        shift_open,
        void_denied: staff.role == Role::Kitchen,
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
    render_detail(&st, &staff, &id, None).await
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
    // Never store a full card number: a reference that looks like a PAN (12+ digits) is refused.
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
            return render_detail(&st, &staff, &id, Some("Uang diterima kurang dari total.".into())).await;
        }
        Some(t)
    } else {
        if reference.is_empty() {
            return render_detail(
                &st,
                &staff,
                &id,
                Some("Nomor referensi wajib untuk pembayaran non-tunai.".into()),
            )
            .await;
        }
        None
    };
    let now = clock::now();
    sqlx::query("INSERT INTO payments (id, order_id, shift_id, method, amount, tendered, reference, cashier_id, created_at) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(rand_id::id()).bind(&id).bind(&shift).bind(&f.method).bind(o.total).bind(tendered)
        .bind(if reference.is_empty() { None } else { Some(&reference) }).bind(&staff.id).bind(now)
        .execute(&mut *tx).await?;
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
    Ok(Redirect::to(&format!("/orders/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct VoidForm {
    reason: String,
    approver: String,
    secret: String,
}

/// Void after payment: needs a reason AND an owner who is not the actor, authenticated by secret.
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
    let approver = match auth::check_credentials(&st.db, &f.approver, &f.secret).await {
        Ok(a) if a.role == Role::Owner && a.id != staff.id => a,
        Ok(_) => {
            return render_detail(
                &st,
                &staff,
                &id,
                Some("Penyetuju harus owner lain, bukan Anda sendiri.".into()),
            )
            .await;
        }
        Err(_) => {
            return render_detail(
                &st,
                &staff,
                &id,
                Some("Kredensial penyetuju salah atau terkunci.".into()),
            )
            .await;
        }
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
        let lines: Vec<(i64, String)> = sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ?")
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

pub async fn parse_manual(
    fields: Vec<(String, String)>,
) -> AppResult<(Vec<(String, i64)>, Option<String>, Option<String>)> {
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
    Ok((parse_qty(fields, 99)?, note, table))
}
