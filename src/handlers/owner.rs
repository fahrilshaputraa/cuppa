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

pub async fn home(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
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
    render(OwnerHome {
        ctx: Ctx::staff(&st, &staff, "home").await?,
        sales: rupiah(sales),
        paid_count: paid,
        void_count: voids,
        shift_open,
    })
}

async fn menu_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let categories: Vec<CategoryRow> =
        sqlx::query_as("SELECT id, name FROM categories WHERE active = 1 ORDER BY sort_order, name")
            .fetch_all(&st.db)
            .await?;
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT m.id, m.name, c.name AS category, m.price, m.active, m.available FROM menu_items m JOIN categories c ON c.id = m.category_id ORDER BY c.name, m.name",
    ).fetch_all(&st.db).await?;
    let items = rows
        .into_iter()
        .map(|r| ItemView {
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
    let users: Vec<UserView> = sqlx::query_as("SELECT id, name, role, active FROM users ORDER BY name")
        .fetch_all(&st.db)
        .await?;
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
    settings_page(&st, &staff, None).await
}

async fn settings_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Html<String>> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    let pct = |bp: i64| {
        if bp % 100 == 0 {
            (bp / 100).to_string()
        } else {
            format!("{}.{:02}", bp / 100, bp % 100)
        }
    };
    render(OwnerSettings {
        ctx: Ctx::staff(st, staff, "settings").await?,
        error,
        tax_pct: pct(s.tax_bp),
        service_pct: pct(s.service_bp),
        s,
    })
}

fn pct_to_bp(s: &str) -> Option<i64> {
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
        )
        .await;
    };
    let name = clean(&f.outlet_name, 60);
    if name.is_empty() {
        return settings_page(&st, &staff, Some("Nama kafe wajib diisi.".into())).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE outlet_settings SET outlet_name=?, tax_bp=?, service_bp=?, tax_inclusive=?, service_in_tax_base=?, qr_ordering=? WHERE id=1")
        .bind(&name).bind(tax).bind(svc)
        .bind(f.tax_inclusive.is_some() as i64).bind(f.service_in_tax_base.is_some() as i64).bind(f.qr_ordering.is_some() as i64)
        .execute(&mut *tx).await?;
    let detail = format!("tax_bp={tax} service_bp={svc}");
    let mut e = Entry::new(Some(&staff.id), "settings.updated", "settings", Some("1"));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    settings_page(&st, &staff, None).await
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
