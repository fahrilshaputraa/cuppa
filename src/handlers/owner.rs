// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, load_settings},
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

pub async fn home(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    let done: i64 = sqlx::query_scalar("SELECT wizard_done FROM outlet_settings WHERE id = 1")
        .fetch_one(&st.db)
        .await?;
    if done == 0 {
        return Ok(Redirect::to("/owner/wizard").into_response());
    }
    let day = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let (sales, paid): (i64, i64) =
        sqlx::query_as("SELECT COALESCE(SUM(total),0), COUNT(*) FROM orders WHERE day = ? AND status = 'paid'")
            .bind(&day)
            .fetch_one(&st.db)
            .await?;
    let voids: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE day = ? AND status = 'voided'")
        .bind(&day)
        .fetch_one(&st.db)
        .await?;
    let shift_open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM shifts WHERE closed_at IS NULL)")
        .fetch_one(&st.db)
        .await?;
    let refunds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(r.amount),0) FROM refunds r JOIN orders o ON o.id = r.order_id WHERE o.day = ?",
    )
    .bind(&day)
    .fetch_one(&st.db)
    .await?;
    Ok(render(OwnerHome {
        ctx: Ctx::staff(&st, &staff, "home").await?,
        sales: rupiah(sales - refunds),
        paid_count: paid,
        void_count: voids,
        shift_open,
    })?
    .into_response())
}

async fn menu_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let categories: Vec<CategoryRow> =
        sqlx::query_as("SELECT id, name FROM categories WHERE active = 1 ORDER BY sort_order, name")
            .fetch_all(&st.db)
            .await?;
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT m.id, m.name, c.name AS category, m.price, m.active, m.available, m.stock FROM menu_items m JOIN categories c ON c.id = m.category_id ORDER BY c.name, m.name",
    ).fetch_all(&st.db).await?;
    let items = rows
        .into_iter()
        .map(|r| ItemView {
            stock: r.stock,
            price: r.price,
            id: r.id,
            name: r.name,
            category: r.category,
            price_fmt: rupiah(r.price),
            active: r.active,
            available: r.available,
        })
        .collect();
    Ok(render(OwnerMenu {
        ctx: Ctx::staff(st, staff, "menu").await?,
        error,
        categories,
        items,
    })?
    .into_response())
}

pub async fn menu(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    menu_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct NameForm {
    name: String,
}

pub async fn add_category(State(st): State<AppState>, staff: Staff, Form(f): Form<NameForm>) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 60);
    if name.is_empty() {
        return menu_page(&st, &staff, Some("Nama kategori wajib diisi.".into())).await;
    }
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("INSERT INTO categories (id, name) VALUES (?, ?)")
        .bind(&id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "category.created", "category", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/menu").into_response())
}

#[derive(Deserialize)]
pub struct ItemForm {
    category_id: String,
    name: String,
    price: i64,
}

pub async fn add_item(State(st): State<AppState>, staff: Staff, Form(f): Form<ItemForm>) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 80);
    if name.is_empty() || !(0..=100_000_000).contains(&f.price) {
        return menu_page(&st, &staff, Some("Nama atau harga tidak valid.".into())).await;
    }
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM categories WHERE id = ? AND active = 1)")
        .bind(&f.category_id)
        .fetch_one(&mut *tx)
        .await?;
    if !ok {
        return menu_page(&st, &staff, Some("Kategori tidak valid.".into())).await;
    }
    sqlx::query("INSERT INTO menu_items (id, category_id, name, price) VALUES (?,?,?,?)")
        .bind(&id)
        .bind(&f.category_id)
        .bind(&name)
        .bind(f.price)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "menu.created", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/menu").into_response())
}

async fn toggle(st: &AppState, staff: &Staff, id: &str, available: bool) -> AppResult<Redirect> {
    owner(staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let sql = if available {
        "UPDATE menu_items SET available = 1 - available WHERE id = ?"
    } else {
        "UPDATE menu_items SET active = 1 - active WHERE id = ?"
    };
    if sqlx::query(sql).bind(id).execute(&mut *tx).await?.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    let action = if available {
        "menu.toggle_available"
    } else {
        "menu.toggle_active"
    };
    audit::record(&mut tx, Entry::new(Some(&staff.id), action, "menu_item", Some(id))).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/menu"))
}

pub async fn toggle_available(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    toggle(&st, &staff, &id, true).await
}
pub async fn toggle_active(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    toggle(&st, &staff, &id, false).await
}

// ---- Tables & QR ----

async fn tables_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let tables: Vec<TableRow> = sqlx::query_as("SELECT id, label, active FROM tables ORDER BY label")
        .fetch_all(&st.db)
        .await?;
    Ok(render(OwnerTables {
        ctx: Ctx::staff(st, staff, "tables").await?,
        error,
        tables,
    })?
    .into_response())
}

pub async fn tables(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    tables_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct LabelForm {
    label: String,
}

pub async fn add_table(State(st): State<AppState>, staff: Staff, Form(f): Form<LabelForm>) -> AppResult<Response> {
    owner(&staff)?;
    let label = clean(&f.label, 20);
    if label.is_empty() {
        return tables_page(&st, &staff, Some("Nama meja wajib diisi.".into())).await;
    }
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    if sqlx::query("INSERT INTO tables (id, label, token) VALUES (?,?,?)")
        .bind(&id)
        .bind(&label)
        .bind(rand_id::hex_token(32))
        .execute(&mut *tx)
        .await
        .is_err()
    {
        return tables_page(&st, &staff, Some("Nama meja sudah dipakai.".into())).await;
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "table.created", "table", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/tables").into_response())
}

pub async fn rotate(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    if sqlx::query("UPDATE tables SET token = ? WHERE id = ?")
        .bind(rand_id::hex_token(32))
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 0
    {
        return Err(AppError::NotFound);
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "table.token_rotated", "table", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/tables"))
}

pub async fn qr(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Html<String>> {
    owner(&staff)?;
    let (label, token): (String, String) = sqlx::query_as("SELECT label, token FROM tables WHERE id = ?")
        .bind(&id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let url = format!("{}/m/{}", st.cfg.public_url.trim_end_matches('/'), token);
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| anyhow::anyhow!("qr: {e}"))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build();
    render(OwnerQr {
        ctx: Ctx::staff(&st, &staff, "tables").await?,
        label,
        svg,
        url,
    })
}

// ---- Staff ----

async fn staff_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let users = super::extras::user_views(st).await?;
    Ok(render(OwnerStaff {
        ctx: Ctx::staff(st, staff, "staff").await?,
        error,
        users,
    })?
    .into_response())
}

pub async fn staff(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    staff_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct NewStaff {
    name: String,
    role: String,
    secret: String,
}

pub async fn add_staff(State(st): State<AppState>, staff: Staff, Form(f): Form<NewStaff>) -> AppResult<Response> {
    owner(&staff)?;
    let role = Role::parse(&f.role).ok_or_else(|| AppError::Invalid("Peran tidak valid.".into()))?;
    let name = clean(&f.name, 60);
    if name.is_empty() {
        return staff_page(&st, &staff, Some("Nama wajib diisi.".into())).await;
    }
    if let Err(e) = auth::validate_secret(role, &f.secret) {
        return staff_page(&st, &staff, Some(e)).await;
    }
    let hash = auth::hash_secret(f.secret).await?;
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    if sqlx::query("INSERT INTO users (id, name, role, pin_hash, created_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(&name)
        .bind(role.as_str())
        .bind(&hash)
        .bind(clock::now())
        .execute(&mut *tx)
        .await
        .is_err()
    {
        return staff_page(&st, &staff, Some("Gagal menambah staf.".into())).await;
    }
    audit::record(&mut tx, Entry::new(Some(&staff.id), "user.created", "user", Some(&id))).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/staff").into_response())
}

pub async fn toggle_staff(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    owner(&staff)?;
    if id == staff.id {
        return staff_page(&st, &staff, Some("Anda tidak bisa menonaktifkan akun sendiri.".into())).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE users SET active = 1 - active WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    // Deactivation must kick the user out immediately.
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(&mut tx, Entry::new(Some(&staff.id), "user.toggled", "user", Some(&id))).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/staff").into_response())
}

// ---- Settings & audit ----

pub async fn settings(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
    settings_page(&st, &staff, None, None).await
}

async fn settings_page(
    st: &AppState,
    staff: &Staff,
    error: Option<String>,
    notice: Option<String>,
) -> AppResult<Html<String>> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    let pct = |bp: i64| {
        if bp % 100 == 0 {
            (bp / 100).to_string()
        } else {
            format!("{}.{:02}", bp / 100, bp % 100)
        }
    };
    let backups = crate::backup::dir_for(&st.cfg.database_url)
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|rd| {
            let mut v: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".db"))
                .collect();
            v.sort();
            v.reverse();
            v.truncate(5);
            v
        })
        .unwrap_or_default();
    render(OwnerSettings {
        ctx: Ctx::staff(st, staff, "settings").await?,
        error,
        notice,
        backups,
        version: env!("CARGO_PKG_VERSION"),
        discount_pct: pct(s.cashier_discount_max_bp),
        tax_pct: pct(s.tax_bp),
        service_pct: pct(s.service_bp),
        s,
    })
}

pub fn pct_text(bp: i64) -> String {
    if bp % 100 == 0 {
        (bp / 100).to_string()
    } else {
        format!("{}.{:02}", bp / 100, bp % 100)
    }
}

pub fn pct_to_bp(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', ".");
    let (i, f) = s.split_once('.').unwrap_or((&s, ""));
    if f.len() > 2 || i.is_empty() || !i.bytes().all(|b| b.is_ascii_digit()) || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let bp = i.parse::<i64>().ok()? * 100 + format!("{f:0<2}").parse::<i64>().ok()?;
    (0..=10_000).contains(&bp).then_some(bp)
}

#[derive(Deserialize)]
pub struct SettingsForm {
    outlet_name: String,
    tax_pct: String,
    service_pct: String,
    tax_inclusive: Option<String>,
    service_in_tax_base: Option<String>,
    qr_ordering: Option<String>,
    printer_host: Option<String>,
    paper_cols: i64,
    receipt_footer: String,
    open_drawer_on_cash: Option<String>,
    kitchen_mode: Option<String>,
    kitchen_printer_host: Option<String>,
    open_from: Option<String>,
    open_to: Option<String>,
    round_to: i64,
    discount_pct: String,
    theme: String,
    require_device: Option<String>,
}

pub async fn save_settings(
    State(st): State<AppState>,
    staff: Staff,
    Form(f): Form<SettingsForm>,
) -> AppResult<Html<String>> {
    owner(&staff)?;
    let (Some(tax), Some(svc)) = (pct_to_bp(&f.tax_pct), pct_to_bp(&f.service_pct)) else {
        return settings_page(
            &st,
            &staff,
            Some("Tarif harus angka 0–100 dengan maksimal 2 desimal.".into()),
            None,
        )
        .await;
    };
    let name = clean(&f.outlet_name, 60);
    if name.is_empty() {
        return settings_page(&st, &staff, Some("Nama kafe wajib diisi.".into()), None).await;
    }
    let printer = clean(f.printer_host.as_deref().unwrap_or(""), 40);
    if !printer.is_empty()
        && let Err(e) = crate::escpos::parse_printer(&printer)
    {
        return settings_page(&st, &staff, Some(e), None).await;
    }
    if ![32, 42, 48].contains(&f.paper_cols) {
        return settings_page(&st, &staff, Some("Lebar kertas tidak valid.".into()), None).await;
    }
    let kprinter = clean(f.kitchen_printer_host.as_deref().unwrap_or(""), 40);
    if !kprinter.is_empty()
        && let Err(e) = crate::escpos::parse_printer(&kprinter)
    {
        return settings_page(&st, &staff, Some(e), None).await;
    }
    let hm_ok = |v: &Option<String>| match v.as_deref().map(str::trim) {
        None | Some("") => Some(None),
        Some(x)
            if x.len() == 5
                && x.as_bytes()[2] == b':'
                && x[..2].parse::<u8>().is_ok_and(|h| h < 24)
                && x[3..].parse::<u8>().is_ok_and(|m| m < 60) =>
        {
            Some(Some(x.to_string()))
        }
        _ => None,
    };
    let (Some(open_from), Some(open_to)) = (hm_ok(&f.open_from), hm_ok(&f.open_to)) else {
        return settings_page(&st, &staff, Some("Jam buka harus berformat HH:MM.".into()), None).await;
    };
    if open_from.is_some() != open_to.is_some() {
        return settings_page(
            &st,
            &staff,
            Some("Isi jam buka dan jam tutup, atau kosongkan keduanya.".into()),
            None,
        )
        .await;
    }
    let Some(disc_bp) = pct_to_bp(&f.discount_pct) else {
        return settings_page(&st, &staff, Some("Batas diskon kasir tidak valid.".into()), None).await;
    };
    if ![0, 100, 500, 1000].contains(&f.round_to) || !["copper", "green", "blue", "rose"].contains(&f.theme.as_str()) {
        return settings_page(&st, &staff, Some("Pembulatan atau tema tidak valid.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE outlet_settings SET kitchen_mode=?, kitchen_printer_host=?, open_from=?, open_to=?, round_to=?, cashier_discount_max_bp=?, theme=?, require_device=? WHERE id=1")
        .bind(f.kitchen_mode.is_some() as i64)
        .bind(if kprinter.is_empty() { None } else { Some(kprinter) })
        .bind(open_from)
        .bind(open_to)
        .bind(f.round_to)
        .bind(disc_bp)
        .bind(&f.theme)
        .bind(f.require_device.is_some() as i64)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE outlet_settings SET outlet_name=?, tax_bp=?, service_bp=?, tax_inclusive=?, service_in_tax_base=?, qr_ordering=?, printer_host=?, paper_cols=?, receipt_footer=?, open_drawer_on_cash=? WHERE id=1")
        .bind(&name).bind(tax).bind(svc)
        .bind(f.tax_inclusive.is_some() as i64).bind(f.service_in_tax_base.is_some() as i64).bind(f.qr_ordering.is_some() as i64)
        .bind(if printer.is_empty() { None } else { Some(printer) }).bind(f.paper_cols).bind(clean(&f.receipt_footer, 80)).bind(f.open_drawer_on_cash.is_some() as i64)
        .execute(&mut *tx).await?;
    let detail = format!("tax_bp={tax} service_bp={svc}");
    let mut e = Entry::new(Some(&staff.id), "settings.updated", "settings", Some("1"));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    settings_page(&st, &staff, None, Some("Pengaturan disimpan.".into())).await
}

pub async fn printer_test(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
    let mut c = st.db.acquire().await?;
    let host = load_settings(&mut c).await?.printer_host;
    drop(c);
    let Some(host) = host else {
        return settings_page(&st, &staff, Some("Isi alamat printer lalu simpan dulu.".into()), None).await;
    };
    let addr = crate::escpos::parse_printer(&host).map_err(AppError::Invalid)?;
    let mut data = vec![0x1b, b'@'];
    data.extend_from_slice(b"Cuppa - tes cetak\nPrinter terhubung.\n\n\n\n");
    data.extend_from_slice(&[0x1d, b'V', 66, 0]);
    data.extend_from_slice(&crate::escpos::drawer_pulse());
    match crate::escpos::send(addr, &data).await {
        Ok(()) => {
            settings_page(
                &st,
                &staff,
                None,
                Some("Tes cetak terkirim (laci kas juga dipicu).".into()),
            )
            .await
        }
        Err(_) => {
            settings_page(
                &st,
                &staff,
                Some("Printer tidak merespons. Cek kabel LAN, alamat IP, dan port 9100.".into()),
                None,
            )
            .await
        }
    }
}

/// Consistent snapshot download. Audited because it exports every sale.
pub async fn backup_download(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    let tmp = std::env::temp_dir().join(format!("cuppa-backup-{}.db", rand_id::hex_token(8)));
    crate::backup::snapshot(&st, &tmp).await?;
    let bytes = tokio::fs::read(&tmp).await?;
    let _ = tokio::fs::remove_file(&tmp).await;
    let mut c = st.db.acquire().await?;
    audit::record(
        &mut c,
        Entry::new(Some(&staff.id), "backup.downloaded", "database", None),
    )
    .await?;
    let name = format!("cuppa-{}.db", clock::day(clock::now(), st.cfg.tz_offset_hours));
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

pub async fn audit_view(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
    let rows: Vec<(i64, Option<String>, String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.at, u.name, a.action, a.entity, a.reason FROM audit_log a LEFT JOIN users u ON u.id = a.actor_id ORDER BY a.id DESC LIMIT 200",
    ).fetch_all(&st.db).await?;
    let tz = st.cfg.tz_offset_hours;
    let rows = rows
        .into_iter()
        .map(|(at, actor, action, entity, reason)| AuditView {
            at: format!("{} {}", clock::day(at, tz), clock::hm(at, tz)),
            actor: actor.unwrap_or_else(|| "—".into()),
            action,
            entity,
            reason: reason.unwrap_or_default(),
        })
        .collect();
    render(OwnerAudit {
        ctx: Ctx::staff(&st, &staff, "audit").await?,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::pct_to_bp;
    #[test]
    fn percent_parsing() {
        assert_eq!(pct_to_bp("10"), Some(1000));
        assert_eq!(pct_to_bp("5,5"), Some(550));
        assert_eq!(pct_to_bp("0.25"), Some(25));
        assert_eq!(pct_to_bp("101"), None);
        assert_eq!(pct_to_bp("abc"), None);
        assert_eq!(pct_to_bp("1.234"), None);
    }
}
