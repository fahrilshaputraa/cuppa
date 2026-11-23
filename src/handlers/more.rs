// SPDX-License-Identifier: AGPL-3.0-or-later
//! Menu photos, kitchen stations, reservations, opt-in loyalty customers and owner 2FA.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, orders},
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::{ConnectInfo, Multipart, Path, Query, State},
    http::{HeaderMap, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{net::SocketAddr, time::Duration};

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

// ---- Photos (FR-02/23) -------------------------------------------------------------------------

/// Decode with hard limits, drop metadata and alpha, shrink and re-encode as JPEG. Never stores the upload as-is.
fn reencode(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "File bukan gambar yang valid")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|_| "File bukan gambar yang valid (gunakan JPG, PNG, atau WebP)")?;
    let img = img.thumbnail(720, 720).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
        .encode_image(&img)
        .map_err(|_| "Gagal memproses gambar")?;
    Ok(out)
}

pub async fn photo_upload(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    mut mp: Multipart,
) -> AppResult<Redirect> {
    owner(&staff)?;
    let dir = st
        .cfg
        .uploads_dir()
        .ok_or_else(|| AppError::Invalid("Penyimpanan foto tidak tersedia pada database ini.".into()))?;
    let mut data = None;
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|_| AppError::Invalid("Unggahan tidak valid.".into()))?
    {
        if field.name() == Some("photo") {
            data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|_| AppError::Invalid("Foto terlalu besar (maksimal 3 MB).".into()))?,
            );
            break;
        }
    }
    let bytes = data
        .filter(|b| !b.is_empty())
        .ok_or_else(|| AppError::Invalid("Pilih file foto.".into()))?;
    if bytes.len() > 3 * 1024 * 1024 {
        return Err(AppError::Invalid("Foto terlalu besar (maksimal 3 MB).".into()));
    }
    let jpeg = tokio::task::spawn_blocking(move || reencode(&bytes))
        .await?
        .map_err(|m| AppError::Invalid(m.into()))?;
    let name = format!("{}.jpg", rand_id::id());
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(&name), jpeg).await?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let old: Option<Option<String>> = sqlx::query_scalar("SELECT photo FROM menu_items WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(old) = old else {
        let _ = tokio::fs::remove_file(dir.join(&name)).await;
        return Err(AppError::NotFound);
    };
    sqlx::query("UPDATE menu_items SET photo = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "menu.photo", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    if let Some(old) = old.filter(|o| valid_upload_name(o)) {
        let _ = tokio::fs::remove_file(dir.join(old)).await;
    }
    Ok(Redirect::to("/owner/menu"))
}

fn valid_upload_name(n: &str) -> bool {
    n.len() == 36
        && n.ends_with(".jpg")
        && n[..32]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Public: customers need the pictures. Only server-generated names are served (no path traversal possible).
pub async fn serve_upload(State(st): State<AppState>, Path(name): Path<String>) -> AppResult<Response> {
    if !valid_upload_name(&name) {
        return Err(AppError::NotFound);
    }
    let dir = st.cfg.uploads_dir().ok_or(AppError::NotFound)?;
    let bytes = tokio::fs::read(dir.join(&name)).await.map_err(|_| AppError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=604800, immutable"),
        ],
        bytes,
    )
        .into_response())
}

// ---- Stations (FR-20) --------------------------------------------------------------------------

async fn stations_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let stations: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, printer_host FROM stations ORDER BY name")
            .fetch_all(&st.db)
            .await?;
    let cats: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, station_id FROM categories WHERE active = 1 ORDER BY sort_order, name")
            .fetch_all(&st.db)
            .await?;
    Ok(render(StationsPage {
        ctx: Ctx::staff(st, staff, "stations").await?,
        error,
        stations: stations
            .iter()
            .map(|(id, name, p)| (id.clone(), name.clone(), p.clone().unwrap_or_default()))
            .collect(),
        categories: cats
            .into_iter()
            .map(|(id, name, sid)| (id, name, sid.unwrap_or_default()))
            .collect(),
    })?
    .into_response())
}

pub async fn stations_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    stations_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct StationNew {
    name: String,
    printer_host: Option<String>,
}

pub async fn stations_add(State(st): State<AppState>, staff: Staff, Form(f): Form<StationNew>) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 30);
    let printer = clean(f.printer_host.as_deref().unwrap_or(""), 40);
    if name.is_empty() {
        return stations_page(&st, &staff, Some("Nama stasiun wajib diisi.".into())).await;
    }
    if !printer.is_empty()
        && let Err(e) = crate::escpos::parse_printer(&printer)
    {
        return stations_page(&st, &staff, Some(e)).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    if sqlx::query("INSERT INTO stations (id, name, printer_host) VALUES (?,?,?)")
        .bind(&id)
        .bind(&name)
        .bind(if printer.is_empty() { None } else { Some(printer) })
        .execute(&mut *tx)
        .await
        .is_err()
    {
        drop(tx);
        return stations_page(&st, &staff, Some("Nama stasiun sudah dipakai.".into())).await;
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "station.created", "station", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/stations").into_response())
}

#[derive(Deserialize)]
pub struct StationPick {
    station_id: String,
}

pub async fn category_station(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<StationPick>,
) -> AppResult<Redirect> {
    owner(&staff)?;
    let sid: Option<String> = if f.station_id.is_empty() {
        None
    } else {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stations WHERE id = ?)")
            .bind(&f.station_id)
            .fetch_one(&st.db)
            .await?;
        if !ok {
            return Err(AppError::Invalid("Stasiun tidak valid.".into()));
        }
        Some(f.station_id)
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE categories SET station_id = ? WHERE id = ?")
        .bind(&sid)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "category.station", "category", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/stations"))
}

#[derive(Deserialize)]
pub struct StationQ {
    station: Option<String>,
}

/// Kitchen screen tickets, optionally limited to the categories mapped to one station.
pub async fn kitchen_orders(st: &AppState, station: Option<&str>) -> AppResult<Vec<QueueOrder>> {
    let mut all = orders::queue_orders(st, &["confirmed"]).await?;
    if let Some(sid) = station.filter(|s| !s.is_empty()) {
        let mut keep = Vec::new();
        for mut o in all.drain(..) {
            let lines: Vec<(i64, String)> = sqlx::query_as(
                "SELECT i.qty, i.name FROM order_items i JOIN menu_items m ON m.id = i.menu_id JOIN categories c ON c.id = m.category_id \
                 WHERE i.order_id = ? AND i.voided = 0 AND c.station_id = ?",
            )
            .bind(&o.id)
            .bind(sid)
            .fetch_all(&st.db)
            .await?;
            if !lines.is_empty() {
                o.lines = lines.into_iter().map(|(q, n)| format!("{q}× {n}")).collect();
                keep.push(o);
            }
        }
        all = keep;
    }
    Ok(all)
}

pub async fn kitchen_tickets(
    State(st): State<AppState>,
    staff: Staff,
    Query(q): Query<StationQ>,
) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Kitchen])?;
    render(KitchenTickets {
        orders: kitchen_orders(&st, q.station.as_deref()).await?,
    })
}

// ---- Reservations (FR-61) ----------------------------------------------------------------------

async fn reservations_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let tz = st.cfg.tz_offset_hours;
    let since = clock::now() - 6 * 3600;
    let rows: Vec<(String, String, Option<String>, i64, i64, i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, phone, at_ts, pax, deposit, status, note FROM reservations WHERE at_ts >= ? ORDER BY at_ts LIMIT 200").bind(since).fetch_all(&st.db).await?;
    let today = clock::day(clock::now(), tz);
    Ok(render(ReservationsPage {
        ctx: Ctx::staff(st, staff, "reservations").await?,
        error,
        today,
        rows: rows
            .into_iter()
            .map(|(id, name, phone, at, pax, deposit, status, note)| ReservationView {
                id,
                name,
                phone: phone.unwrap_or_default(),
                when: format!("{} {}", clock::day(at, tz), clock::hm(at, tz)),
                pax,
                deposit: if deposit > 0 { rupiah(deposit) } else { "—".into() },
                status: status.clone(),
                status_label: match status.as_str() {
                    "arrived" => "Hadir",
                    "cancelled" => "Batal",
                    _ => "Dipesan",
                },
                note: note.unwrap_or_default(),
            })
            .collect(),
    })?
    .into_response())
}

pub async fn reservations_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    reservations_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct ReservationNew {
    name: String,
    phone: Option<String>,
    day: String,
    time: String,
    pax: i64,
    deposit: Option<i64>,
    note: Option<String>,
}

pub async fn reservations_add(
    State(st): State<AppState>,
    staff: Staff,
    Form(f): Form<ReservationNew>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let name = clean(&f.name, 60);
    let at = clock::local_to_ts(&f.day, &f.time, st.cfg.tz_offset_hours);
    let deposit = f.deposit.unwrap_or(0);
    let phone = clean(f.phone.as_deref().unwrap_or(""), 20);
    if name.is_empty()
        || at.is_none()
        || !(1..=200).contains(&f.pax)
        || !(0..=100_000_000).contains(&deposit)
        || !phone
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'+' || b == b' ' || b == b'-')
    {
        return reservations_page(
            &st,
            &staff,
            Some("Nama, tanggal, jam, dan jumlah tamu wajib diisi dengan benar.".into()),
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    sqlx::query(
        "INSERT INTO reservations (id, name, phone, at_ts, pax, deposit, note, created_at) VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(if phone.is_empty() { None } else { Some(phone) })
    .bind(at)
    .bind(f.pax)
    .bind(deposit)
    .bind(f.note.as_deref().map(|n| clean(n, 120)).filter(|n| !n.is_empty()))
    .bind(clock::now())
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "reservation.created", "reservation", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/cashier/reservations").into_response())
}

#[derive(Deserialize)]
pub struct ResvStatus {
    status: String,
}

pub async fn reservation_status(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<ResvStatus>,
) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    if !["booked", "arrived", "cancelled"].contains(&f.status.as_str()) {
        return Err(AppError::Invalid("Status tidak valid.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE reservations SET status = ? WHERE id = ?")
        .bind(&f.status)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let action = format!("reservation.{}", f.status);
    audit::record(&mut tx, Entry::new(Some(&staff.id), &action, "reservation", Some(&id))).await?;
    tx.commit().await?;
    Ok(Redirect::to("/cashier/reservations"))
}

// ---- Loyalty customers, opt-in with consent (FR-57) --------------------------------------------

#[derive(Deserialize)]
pub struct JoinForm {
    name: String,
    phone: String,
    consent: Option<String>,
}

pub async fn loyalty_join(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((token, id)): Path<(String, String)>,
    Form(f): Form<JoinForm>,
) -> AppResult<Response> {
    let ip = crate::security::client_ip(&st, &headers, peer);
    if !st.limiter.allow(&format!("join-ip:{ip}"), 8, Duration::from_secs(600)) {
        return Err(AppError::TooMany);
    }
    if f.consent.is_none() {
        return Err(AppError::Invalid("Centang persetujuan untuk bergabung.".into()));
    }
    let name = clean(&f.name, 60);
    let phone: String = f
        .phone
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let digits = phone.trim_start_matches('+');
    if name.is_empty() || !(8..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::Invalid(
            "Nama dan nomor HP (8–15 digit) wajib diisi dengan benar.".into(),
        ));
    }
    let table_id: String = sqlx::query_scalar("SELECT id FROM tables WHERE token = ? AND active = 1")
        .bind(&token)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM orders WHERE id = ? AND table_id = ? AND status = 'paid')")
            .bind(&id)
            .bind(&table_id)
            .fetch_one(&mut *tx)
            .await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    let cid: String = match sqlx::query_scalar("SELECT id FROM customers WHERE phone = ?")
        .bind(&phone)
        .fetch_optional(&mut *tx)
        .await?
    {
        Some(c) => c,
        None => {
            let c = rand_id::id();
            sqlx::query("INSERT INTO customers (id, name, phone, consent_at, created_at) VALUES (?,?,?,?,?)")
                .bind(&c)
                .bind(&name)
                .bind(&phone)
                .bind(clock::now())
                .bind(clock::now())
                .execute(&mut *tx)
                .await?;
            c
        }
    };
    sqlx::query("INSERT OR IGNORE INTO order_customers (order_id, customer_id) VALUES (?, ?)")
        .bind(&id)
        .bind(&cid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/m/{token}/orders/{id}")).into_response())
}

pub async fn customers_get(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    owner(&staff)?;
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT c.id, c.name, c.phone, COUNT(o.id), COALESCE(SUM(o.total),0) FROM customers c LEFT JOIN order_customers oc ON oc.customer_id = c.id LEFT JOIN orders o ON o.id = oc.order_id AND o.status = 'paid' GROUP BY c.id ORDER BY 5 DESC LIMIT 300",
    )
    .fetch_all(&st.db)
    .await?;
    let mut out = Vec::new();
    for (id, name, phone, visits, spend) in rows {
        let fav: Option<String> = sqlx::query_scalar(
            "SELECT i.name FROM order_items i JOIN orders o ON o.id = i.order_id JOIN order_customers oc ON oc.order_id = o.id WHERE oc.customer_id = ? AND o.status = 'paid' AND i.voided = 0 GROUP BY i.name ORDER BY SUM(i.qty) DESC LIMIT 1",
        )
        .bind(&id)
        .fetch_optional(&st.db)
        .await?;
        out.push(CustomerView {
            id,
            name,
            phone,
            visits,
            spend: rupiah(spend),
            points: spend / 10_000,
            favorite: fav.unwrap_or_else(|| "—".into()),
        });
    }
    render(CustomersPage {
        ctx: Ctx::staff(&st, &staff, "customers").await?,
        rows: out,
    })
}

/// Right to erasure: removes the person and unlinks their orders (sales figures stay).
pub async fn customer_erase(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("DELETE FROM order_customers WHERE customer_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM customers WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "customer.erased", "customer", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/customers"))
}

// ---- Owner 2FA (FR-51) -------------------------------------------------------------------------

async fn security_page(
    st: &AppState,
    staff: &Staff,
    error: Option<String>,
    notice: Option<String>,
) -> AppResult<Response> {
    let (secret, pending): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT totp_secret, totp_pending FROM users WHERE id = ?")
            .bind(&staff.id)
            .fetch_one(&st.db)
            .await?;
    let enabled = secret.is_some();
    let (setup_qr, setup_key) = if enabled {
        (String::new(), String::new())
    } else {
        let pend = match pending {
            Some(p) => p,
            None => {
                let p = crate::totp::new_secret_hex();
                sqlx::query("UPDATE users SET totp_pending = ? WHERE id = ?")
                    .bind(&p)
                    .bind(&staff.id)
                    .execute(&st.db)
                    .await?;
                p
            }
        };
        let uri = crate::totp::otpauth_uri(&staff.name, "Cuppa", &pend);
        let code = qrcode::QrCode::new(uri.as_bytes()).map_err(|e| anyhow::anyhow!("qr: {e}"))?;
        (
            code.render::<qrcode::render::svg::Color>()
                .min_dimensions(200, 200)
                .quiet_zone(true)
                .build(),
            crate::totp::base32(&hex::decode(&pend).unwrap_or_default()),
        )
    };
    Ok(render(SecurityPage {
        ctx: Ctx::staff(st, staff, "security").await?,
        error,
        notice,
        enabled,
        setup_qr,
        setup_key,
    })?
    .into_response())
}

pub async fn security_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    security_page(&st, &staff, None, None).await
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

pub async fn security_enable(State(st): State<AppState>, staff: Staff, Form(f): Form<CodeForm>) -> AppResult<Response> {
    owner(&staff)?;
    let pending: Option<String> = sqlx::query_scalar("SELECT totp_pending FROM users WHERE id = ?")
        .bind(&staff.id)
        .fetch_one(&st.db)
        .await?;
    let Some(pending) = pending else {
        return security_page(&st, &staff, Some("Mulai ulang pengaturan.".into()), None).await;
    };
    let Some(step) = crate::totp::verify(&pending, &f.code, clock::now(), 0) else {
        return security_page(
            &st,
            &staff,
            Some("Kode salah. Periksa jam perangkat dan coba lagi.".into()),
            None,
        )
        .await;
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE users SET totp_secret = totp_pending, totp_pending = NULL, totp_last = ? WHERE id = ?")
        .bind(step)
        .bind(&staff.id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "auth.2fa_enabled", "user", Some(&staff.id)),
    )
    .await?;
    tx.commit().await?;
    security_page(&st, &staff, None, Some("Verifikasi dua langkah aktif.".into())).await
}

pub async fn security_disable(
    State(st): State<AppState>,
    staff: Staff,
    Form(f): Form<CodeForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    let (secret, last): (Option<String>, i64) = sqlx::query_as("SELECT totp_secret, totp_last FROM users WHERE id = ?")
        .bind(&staff.id)
        .fetch_one(&st.db)
        .await?;
    let Some(secret) = secret else {
        return security_page(&st, &staff, None, None).await;
    };
    if crate::totp::verify(&secret, &f.code, clock::now(), last).is_none() {
        return security_page(&st, &staff, Some("Kode salah.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE users SET totp_secret = NULL, totp_pending = NULL WHERE id = ?")
        .bind(&staff.id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "auth.2fa_disabled", "user", Some(&staff.id)),
    )
    .await?;
    tx.commit().await?;
    security_page(&st, &staff, None, Some("Verifikasi dua langkah dimatikan.".into())).await
}

// ---- Scheduled prices (FR-49) ------------------------------------------------------------------

async fn prices_page(st: &AppState, staff: &Staff, menu_id: &str, error: Option<String>) -> AppResult<Response> {
    let (name, price): (String, i64) = sqlx::query_as("SELECT name, price FROM menu_items WHERE id = ?")
        .bind(menu_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let rules: Vec<(String, i64, String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, price, from_hm, to_hm, label, active FROM price_rules WHERE menu_id = ? ORDER BY from_hm",
    )
    .bind(menu_id)
    .fetch_all(&st.db)
    .await?;
    Ok(render(PricesPage {
        ctx: Ctx::staff(st, staff, "menu").await?,
        error,
        menu_id: menu_id.into(),
        menu_name: name,
        list_price: rupiah(price),
        rules: rules
            .into_iter()
            .map(|(id, p, f, t, l, a)| (id, rupiah(p), format!("{f}–{t}"), l.unwrap_or_default(), a == 1))
            .collect(),
    })?
    .into_response())
}

pub async fn prices_get(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    owner(&staff)?;
    prices_page(&st, &staff, &id, None).await
}

#[derive(Deserialize)]
pub struct PriceRuleNew {
    price: i64,
    from_hm: String,
    to_hm: String,
    label: Option<String>,
}

fn hm_valid(x: &str) -> bool {
    x.len() == 5
        && x.as_bytes()[2] == b':'
        && x[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && x[3..].parse::<u8>().is_ok_and(|m| m < 60)
}

pub async fn prices_add(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<PriceRuleNew>,
) -> AppResult<Response> {
    owner(&staff)?;
    if !(0..=100_000_000).contains(&f.price) || !hm_valid(&f.from_hm) || !hm_valid(&f.to_hm) || f.from_hm == f.to_hm {
        return prices_page(
            &st,
            &staff,
            &id,
            Some("Harga dan jam (HH:MM, awal berbeda dari akhir) wajib diisi dengan benar.".into()),
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM menu_items WHERE id = ?)")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    let rid = rand_id::id();
    sqlx::query("INSERT INTO price_rules (id, menu_id, price, from_hm, to_hm, label) VALUES (?,?,?,?,?,?)")
        .bind(&rid)
        .bind(&id)
        .bind(f.price)
        .bind(&f.from_hm)
        .bind(&f.to_hm)
        .bind(f.label.as_deref().map(|l| clean(l, 30)).filter(|l| !l.is_empty()))
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "price_rule.created", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{id}/prices")).into_response())
}

pub async fn prices_toggle(State(st): State<AppState>, staff: Staff, Path(rid): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let menu_id: String = sqlx::query_scalar("SELECT menu_id FROM price_rules WHERE id = ?")
        .bind(&rid)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE price_rules SET active = 1 - active WHERE id = ?")
        .bind(&rid)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "price_rule.toggled", "menu_item", Some(&menu_id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{menu_id}/prices")))
}

// ---- Appearance: sanitised CSS + HTML banner (FR-50) --------------------------------------------

async fn appearance_page(
    st: &AppState,
    staff: &Staff,
    error: Option<String>,
    notice: Option<String>,
) -> AppResult<Response> {
    let (css, html): (String, String) =
        sqlx::query_as("SELECT custom_css, custom_banner_html FROM outlet_settings WHERE id = 1")
            .fetch_one(&st.db)
            .await?;
    Ok(render(AppearancePage {
        ctx: Ctx::staff(st, staff, "appearance").await?,
        error,
        notice,
        css,
        html,
    })?
    .into_response())
}

pub async fn appearance_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    appearance_page(&st, &staff, None, None).await
}

#[derive(Deserialize)]
pub struct AppearanceForm {
    css: String,
    html: String,
}

pub async fn appearance_post(
    State(st): State<AppState>,
    staff: Staff,
    Form(f): Form<AppearanceForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    let (css, dropped) = crate::custom::sanitize_css(&f.css);
    let html = crate::custom::sanitize_html(&f.html);
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE outlet_settings SET custom_css = ?, custom_banner_html = ? WHERE id = 1")
        .bind(&css)
        .bind(&html)
        .execute(&mut *tx)
        .await?;
    let detail = format!("css {} bytes, {dropped} dropped; html {} bytes", css.len(), html.len());
    let mut e = Entry::new(Some(&staff.id), "appearance.updated", "settings", Some("1"));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    let note = if dropped > 0 {
        format!("Disimpan. {dropped} aturan/properti tidak aman atau tidak didukung dibuang.")
    } else {
        "Disimpan.".to_string()
    };
    appearance_page(&st, &staff, None, Some(note)).await
}

/// Public stylesheet with the owner's sanitised rules, all scoped under `.cust` (customer page only).
pub async fn custom_css(State(st): State<AppState>) -> AppResult<Response> {
    let css: String = sqlx::query_scalar("SELECT custom_css FROM outlet_settings WHERE id = 1")
        .fetch_one(&st.db)
        .await?;
    // Sanitise again on the way out: stored data is never trusted to still be clean.
    let (clean_css, _) = crate::custom::sanitize_css(&css.replace(".cust ", ""));
    Ok((
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        clean_css,
    )
        .into_response())
}
