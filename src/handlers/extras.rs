// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wizard, device registry, modifiers, menu CSV import, item edit, permission overrides, history, cash movements.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, Perm, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, load_settings, orders::order_status_meta},
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

pub const DEVICE_COOKIE: &str = "cuppa_device";

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

// ---- Setup wizard (FR-27) ----------------------------------------------------------------------

async fn wizard_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Html<String>> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    render(WizardPage {
        ctx: Ctx::staff(st, staff, "home").await?,
        error,
        tax_pct: super::owner::pct_text(s.tax_bp),
        s,
    })
}

pub async fn wizard_get(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
    wizard_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct WizardForm {
    qr_ordering: i64,
    kitchen_mode: i64,
    tax_pct: String,
    require_device: Option<String>,
}

pub async fn wizard_post(State(st): State<AppState>, staff: Staff, Form(f): Form<WizardForm>) -> AppResult<Response> {
    owner(&staff)?;
    let Some(tax) = super::owner::pct_to_bp(&f.tax_pct) else {
        return Ok(wizard_page(&st, &staff, Some("Tarif pajak harus angka 0–100.".into()))
            .await?
            .into_response());
    };
    if ![0, 1].contains(&f.qr_ordering) || ![0, 1].contains(&f.kitchen_mode) {
        return Err(AppError::Invalid("Pilihan tidak valid.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE outlet_settings SET qr_ordering=?, kitchen_mode=?, tax_bp=?, require_device=?, wizard_done=1 WHERE id=1")
        .bind(f.qr_ordering)
        .bind(f.kitchen_mode)
        .bind(tax)
        .bind(f.require_device.is_some() as i64)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "wizard.completed", "settings", Some("1")),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner").into_response())
}

// ---- Device registry (FR-32) -------------------------------------------------------------------

pub async fn device_ok(st: &AppState, headers: &HeaderMap) -> bool {
    let Some(tok) = auth::cookie_value(headers, DEVICE_COOKIE) else {
        return false;
    };
    let h = auth::hash_token(tok);
    let ok: Option<String> = sqlx::query_scalar("SELECT id FROM devices WHERE token_hash = ? AND revoked = 0")
        .bind(&h)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    if ok.is_some() {
        let _ = sqlx::query("UPDATE devices SET last_seen = ? WHERE token_hash = ?")
            .bind(clock::now())
            .bind(&h)
            .execute(&st.db)
            .await;
    }
    ok.is_some()
}

async fn devices_page(
    st: &AppState,
    staff: &Staff,
    headers: &HeaderMap,
    error: Option<String>,
    notice: Option<String>,
) -> AppResult<Html<String>> {
    let rows: Vec<(String, String, i64, Option<i64>, i64)> =
        sqlx::query_as("SELECT id, name, created_at, last_seen, revoked FROM devices ORDER BY created_at DESC")
            .fetch_all(&st.db)
            .await?;
    let tz = st.cfg.tz_offset_hours;
    let fmt = |t: i64| format!("{} {}", clock::day(t, tz), clock::hm(t, tz));
    let required: i64 = sqlx::query_scalar("SELECT require_device FROM outlet_settings WHERE id = 1")
        .fetch_one(&st.db)
        .await?;
    render(DevicesPage {
        ctx: Ctx::staff(st, staff, "devices").await?,
        error,
        notice,
        devices: rows
            .into_iter()
            .map(|(id, name, c, l, r)| DeviceView {
                id,
                name,
                created: fmt(c),
                last_seen: l.map(fmt).unwrap_or_else(|| "—".into()),
                revoked: r == 1,
            })
            .collect(),
        this_registered: device_ok(st, headers).await,
        required: required == 1,
    })
}

pub async fn devices_get(State(st): State<AppState>, staff: Staff, headers: HeaderMap) -> AppResult<Html<String>> {
    owner(&staff)?;
    devices_page(&st, &staff, &headers, None, None).await
}

#[derive(Deserialize)]
pub struct DeviceForm {
    name: String,
}

pub async fn devices_add(
    State(st): State<AppState>,
    staff: Staff,
    headers: HeaderMap,
    Form(f): Form<DeviceForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 40);
    if name.is_empty() {
        return Ok(
            devices_page(&st, &staff, &headers, Some("Nama perangkat wajib diisi.".into()), None)
                .await?
                .into_response(),
        );
    }
    let token = rand_id::hex_token(32);
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("INSERT INTO devices (id, name, token_hash, created_by, created_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(&name)
        .bind(auth::hash_token(&token))
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "device.registered", "device", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    let secure = if st.cfg.cookie_secure { "; Secure" } else { "" };
    let cookie = format!("{DEVICE_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000{secure}");
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to("/owner/devices")).into_response())
}

pub async fn devices_revoke(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE devices SET revoked = 1 WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "device.revoked", "device", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/devices"))
}

// ---- Modifiers (FR-03) -------------------------------------------------------------------------

async fn modifiers_page(st: &AppState, staff: &Staff, item_id: &str, error: Option<String>) -> AppResult<Response> {
    let name: String = sqlx::query_scalar("SELECT name FROM menu_items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let groups: Vec<(String, String, i64, i64)> =
        sqlx::query_as("SELECT id, name, required, max_select FROM modifier_groups WHERE menu_id = ? ORDER BY name")
            .bind(item_id)
            .fetch_all(&st.db)
            .await?;
    let mut out = Vec::new();
    for (id, gname, required, max_select) in groups {
        let mods: Vec<(String, String, i64, i64)> =
            sqlx::query_as("SELECT id, name, price, active FROM modifiers WHERE group_id = ? ORDER BY price, name")
                .bind(&id)
                .fetch_all(&st.db)
                .await?;
        out.push(GroupView {
            id,
            name: gname,
            required: required == 1,
            max_select,
            mods: mods
                .into_iter()
                .map(|(id, name, price, a)| ModView {
                    id,
                    name,
                    price,
                    price_fmt: if price > 0 {
                        format!("+{}", rupiah(price))
                    } else {
                        "—".into()
                    },
                    active: a == 1,
                })
                .collect(),
        });
    }
    Ok(render(ModifiersPage {
        ctx: Ctx::staff(st, staff, "menu").await?,
        error,
        item_id: item_id.into(),
        item_name: name,
        groups: out,
    })?
    .into_response())
}

pub async fn modifiers_get(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    owner(&staff)?;
    modifiers_page(&st, &staff, &id, None).await
}

#[derive(Deserialize)]
pub struct GroupForm {
    name: String,
    max_select: Option<i64>,
    required: Option<String>,
}

pub async fn group_add(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<GroupForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 40);
    let max = f.max_select.unwrap_or(1);
    if name.is_empty() || !(1..=10).contains(&max) {
        return modifiers_page(&st, &staff, &id, Some("Nama wajib dan maksimal pilihan 1–10.".into())).await;
    }
    let gid = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM menu_items WHERE id = ?)")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    sqlx::query("INSERT INTO modifier_groups (id, menu_id, name, required, max_select) VALUES (?,?,?,?,?)")
        .bind(&gid)
        .bind(&id)
        .bind(&name)
        .bind(f.required.is_some() as i64)
        .bind(max)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "modifier_group.created", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{id}/modifiers")).into_response())
}

#[derive(Deserialize)]
pub struct ModForm {
    name: String,
    price: i64,
}

pub async fn modifier_add(
    State(st): State<AppState>,
    staff: Staff,
    Path(gid): Path<String>,
    Form(f): Form<ModForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 40);
    let menu_id: String = sqlx::query_scalar("SELECT menu_id FROM modifier_groups WHERE id = ?")
        .bind(&gid)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    if name.is_empty() || !(0..=10_000_000).contains(&f.price) {
        return modifiers_page(&st, &staff, &menu_id, Some("Nama wajib dan harga tidak valid.".into())).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("INSERT INTO modifiers (id, group_id, name, price) VALUES (?,?,?,?)")
        .bind(rand_id::id())
        .bind(&gid)
        .bind(&name)
        .bind(f.price)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "modifier.created", "modifier_group", Some(&gid)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{menu_id}/modifiers")).into_response())
}

pub async fn modifier_toggle(State(st): State<AppState>, staff: Staff, Path(mid): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let menu_id: String = sqlx::query_scalar(
        "SELECT g.menu_id FROM modifiers m JOIN modifier_groups g ON g.id = m.group_id WHERE m.id = ?",
    )
    .bind(&mid)
    .fetch_optional(&st.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE modifiers SET active = 1 - active WHERE id = ?")
        .bind(&mid)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "modifier.toggled", "modifier", Some(&mid)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{menu_id}/modifiers")))
}

// ---- Menu: edit item + CSV import + quick sold-out ----------------------------------------------

#[derive(Deserialize)]
pub struct EditItem {
    name: String,
    price: i64,
    cost: Option<i64>,
    stock: Option<String>,
}

pub async fn item_edit(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<EditItem>,
) -> AppResult<Redirect> {
    owner(&staff)?;
    let name = clean(&f.name, 80);
    let stock: Option<i64> = match f.stock.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(
            s.parse()
                .ok()
                .filter(|v| (0..=1_000_000).contains(v))
                .ok_or_else(|| AppError::Invalid("Stok tidak valid.".into()))?,
        ),
    };
    if name.is_empty() || !(0..=100_000_000).contains(&f.price) {
        return Err(AppError::Invalid("Nama atau harga tidak valid.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let old: (i64, String) = sqlx::query_as("SELECT price, name FROM menu_items WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE menu_items SET name=?, price=?, cost=?, stock=?, available = CASE WHEN ? IS NOT NULL AND ? = 0 THEN 0 ELSE available END WHERE id=?")
        .bind(&name)
        .bind(f.price)
        .bind(f.cost.unwrap_or(0).clamp(0, 100_000_000))
        .bind(stock)
        .bind(stock)
        .bind(stock)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let detail = format!("price {} -> {}; name '{}' -> '{}'", old.0, f.price, old.1, name);
    let mut e = Entry::new(Some(&staff.id), "menu.edited", "menu_item", Some(&id));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/menu"))
}

#[derive(Deserialize)]
pub struct CsvForm {
    csv: String,
}

/// `category,name,price[,stock]` per line. Existing (category,name) pairs get their price/stock updated.
pub async fn menu_import(State(st): State<AppState>, staff: Staff, Form(f): Form<CsvForm>) -> AppResult<Response> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let (mut created, mut updated) = (0, 0);
    for (n, line) in f.csv.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() < 3 || cols.len() > 4 || cols.iter().any(|c| c.contains('"')) {
            if n == 0 {
                continue; // tolerate a header row
            }
            return Err(AppError::Invalid(format!(
                "Baris {} tidak valid. Format: kategori,nama,harga[,stok] (tanpa tanda kutip).",
                n + 1
            )));
        }
        let Ok(price) = cols[2].parse::<i64>() else {
            if n == 0 {
                continue;
            }
            return Err(AppError::Invalid(format!("Baris {}: harga harus angka.", n + 1)));
        };
        let stock: Option<i64> = cols
            .get(3)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<i64>())
            .transpose()
            .map_err(|_| AppError::Invalid(format!("Baris {}: stok harus angka.", n + 1)))?;
        let (cat, name) = (clean(cols[0], 60), clean(cols[1], 80));
        if cat.is_empty() || name.is_empty() || !(0..=100_000_000).contains(&price) {
            return Err(AppError::Invalid(format!("Baris {}: data tidak valid.", n + 1)));
        }
        let cat_id: String = match sqlx::query_scalar("SELECT id FROM categories WHERE name = ? COLLATE NOCASE")
            .bind(&cat)
            .fetch_optional(&mut *tx)
            .await?
        {
            Some(id) => id,
            None => {
                let id = rand_id::id();
                sqlx::query("INSERT INTO categories (id, name) VALUES (?,?)")
                    .bind(&id)
                    .bind(&cat)
                    .execute(&mut *tx)
                    .await?;
                id
            }
        };
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM menu_items WHERE category_id = ? AND name = ? COLLATE NOCASE")
                .bind(&cat_id)
                .bind(&name)
                .fetch_optional(&mut *tx)
                .await?;
        match existing {
            Some(id) => {
                sqlx::query("UPDATE menu_items SET price = ?, stock = ?, active = 1 WHERE id = ?")
                    .bind(price)
                    .bind(stock)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                updated += 1;
            }
            None => {
                sqlx::query("INSERT INTO menu_items (id, category_id, name, price, stock) VALUES (?,?,?,?,?)")
                    .bind(rand_id::id())
                    .bind(&cat_id)
                    .bind(&name)
                    .bind(price)
                    .bind(stock)
                    .execute(&mut *tx)
                    .await?;
                created += 1;
            }
        }
    }
    let detail = format!("created {created}, updated {updated}");
    let mut e = Entry::new(Some(&staff.id), "menu.imported", "menu_item", None);
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/menu").into_response())
}

/// Cashier/kitchen can flip "sold out" quickly (FR-22).
pub async fn sold_out(
    State(st): State<AppState>,
    staff: Staff,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Redirect> {
    staff.need(Perm::SoldOut)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    if sqlx::query("UPDATE menu_items SET available = 1 - available WHERE id = ?")
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
        Entry::new(Some(&staff.id), "menu.toggle_available", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    let back = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.split("://").nth(1))
        .and_then(|r| r.find('/').map(|i| r[i..].to_string()))
        .filter(|p| p.starts_with('/') && !p.starts_with("//"))
        .unwrap_or_else(|| "/cashier/new".into());
    Ok(Redirect::to(&back))
}

// ---- Permission overrides (FR-31) --------------------------------------------------------------

pub async fn perms_save(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Redirect> {
    owner(&staff)?;
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let role = Role::parse(&role).ok_or(AppError::NotFound)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("DELETE FROM user_permission_overrides WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for p in Perm::ALL {
        let want = fields.iter().any(|(k, _)| k == &format!("perm_{}", p.key()));
        if want != p.default_for(role) {
            sqlx::query("INSERT INTO user_permission_overrides (user_id, perm, allowed) VALUES (?,?,?)")
                .bind(&id)
                .bind(p.key())
                .bind(want as i64)
                .execute(&mut *tx)
                .await?;
        }
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "user.permissions_changed", "user", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/staff"))
}

pub async fn user_views(st: &AppState) -> AppResult<Vec<UserView>> {
    let users: Vec<UserRow> = sqlx::query_as("SELECT id, name, role, active FROM users ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    let mut out = Vec::new();
    for u in users {
        let role = Role::parse(&u.role).unwrap_or(Role::Cashier);
        let ov: Vec<(String, i64)> =
            sqlx::query_as("SELECT perm, allowed FROM user_permission_overrides WHERE user_id = ?")
                .bind(&u.id)
                .fetch_all(&st.db)
                .await?;
        let perms = Perm::ALL
            .into_iter()
            .map(|p| {
                let on = ov
                    .iter()
                    .find(|(k, _)| k == p.key())
                    .map(|(_, a)| *a == 1)
                    .unwrap_or_else(|| p.default_for(role));
                (p.key(), p.label(), on)
            })
            .collect();
        out.push(UserView {
            is_owner: role == Role::Owner,
            id: u.id,
            name: u.name,
            role: u.role,
            active: u.active,
            perms,
        });
    }
    Ok(out)
}

// ---- History (FR-19) ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct HistQuery {
    day: Option<String>,
    q: Option<String>,
}

pub async fn history(State(st): State<AppState>, staff: Staff, Query(q): Query<HistQuery>) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    staff.need(Perm::History)?;
    let tz = st.cfg.tz_offset_hours;
    let day = q
        .day
        .filter(|d| d.len() == 10 && d.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .unwrap_or_else(|| clock::day(clock::now(), tz));
    let term = clean(q.q.as_deref().unwrap_or(""), 20);
    let like = format!("%{}%", term.replace(['%', '_'], ""));
    let rows: Vec<(String, i64, i64, Option<String>, String, i64)> = sqlx::query_as(
        "SELECT o.id, o.number, o.created_at, t.label, o.status, o.total FROM orders o LEFT JOIN tables t ON t.id = o.table_id \
         WHERE o.day = ?1 AND (?2 = '%%' OR CAST(o.number AS TEXT) LIKE ?2 OR t.label LIKE ?2) ORDER BY o.number DESC LIMIT 200",
    )
    .bind(&day)
    .bind(&like)
    .fetch_all(&st.db)
    .await?;
    let rows = rows
        .into_iter()
        .map(|(id, number, at, label, status, total)| {
            let (sl, sc) = order_status_meta(&status);
            HistRow {
                id,
                number,
                time: clock::hm(at, tz),
                where_label: label.map(|l| format!("Meja {l}")).unwrap_or_else(|| "Antre".into()),
                status_label: sl,
                status_class: sc,
                total: rupiah(total),
            }
        })
        .collect();
    render(HistoryPage {
        ctx: Ctx::staff(&st, &staff, "history").await?,
        q: term,
        day,
        rows,
    })
}

// ---- Cash in / out / open drawer (FR-17) -------------------------------------------------------

#[derive(Deserialize)]
pub struct CashForm {
    kind: String,
    amount: Option<i64>,
    reason: String,
}

pub async fn cash_move(State(st): State<AppState>, staff: Staff, Form(f): Form<CashForm>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    staff.need(Perm::CashMove)?;
    let reason = clean(&f.reason, 120);
    let amount = f.amount.unwrap_or(0);
    if reason.is_empty()
        || !["in", "out", "open_drawer"].contains(&f.kind.as_str())
        || !(0..=1_000_000_000).contains(&amount)
        || (f.kind != "open_drawer" && amount == 0)
    {
        return Err(AppError::Invalid("Jenis, nominal, dan alasan wajib diisi.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let shift: String = sqlx::query_scalar("SELECT id FROM shifts WHERE closed_at IS NULL")
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::Invalid("Buka shift dulu.".into()))?;
    let id = rand_id::id();
    sqlx::query(
        "INSERT INTO cash_movements (id, shift_id, kind, amount, reason, user_id, created_at) VALUES (?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&shift)
    .bind(&f.kind)
    .bind(amount)
    .bind(&reason)
    .bind(&staff.id)
    .bind(clock::now())
    .execute(&mut *tx)
    .await?;
    let action = format!("cash.{}", f.kind);
    let mut e = Entry::new(Some(&staff.id), &action, "shift", Some(&shift));
    e.reason = Some(&reason);
    let detail = amount.to_string();
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    if f.kind == "open_drawer" {
        super::receipt::pulse_drawer(st.clone());
    }
    Ok(Redirect::to("/cashier/shift"))
}

// ---- Lock (idle screen lock, FR-33) -----------------------------------------------------------

pub async fn lock(State(st): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(t) = auth::cookie_value(&headers, auth::COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(auth::hash_token(t))
            .execute(&st.db)
            .await?;
    }
    Ok((
        [(header::SET_COOKIE, auth::clear_cookie(st.cfg.cookie_secure))],
        Redirect::to("/login"),
    )
        .into_response())
}
