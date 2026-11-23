// SPDX-License-Identifier: AGPL-3.0-or-later
//! Floor plan, order merge, read-only API keys + JSON API, and the in-app ledger.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, orders},
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

// ---- Floor plan (FR-53) ------------------------------------------------------------------------

pub async fn floor(State(st): State<AppState>, staff: Staff) -> AppResult<axum::response::Html<String>> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let tables: Vec<(String, String)> = sqlx::query_as("SELECT id, label FROM tables WHERE active = 1 ORDER BY label")
        .fetch_all(&st.db)
        .await?;
    let mut tiles = Vec::new();
    for (id, label) in tables {
        let open: Vec<(String, i64)> = sqlx::query_as(
            "SELECT id, total FROM orders WHERE table_id = ? AND status IN ('pending','confirmed','ready')",
        )
        .bind(&id)
        .fetch_all(&st.db)
        .await?;
        let call: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM table_calls WHERE table_id = ? AND handled_at IS NULL)")
                .bind(&id)
                .fetch_one(&st.db)
                .await?;
        let total: i64 = open.iter().map(|o| o.1).sum();
        let link = match open.as_slice() {
            [] => String::new(),
            [one] => format!("/orders/{}", one.0),
            _ => format!("/cashier/history?q={label}"),
        };
        tiles.push(FloorTile {
            label,
            count: open.len() as i64,
            total: if total > 0 { rupiah(total) } else { String::new() },
            call,
            link,
        });
    }
    render(FloorPage {
        ctx: Ctx::staff(&st, &staff, "floor").await?,
        tiles,
    })
}

#[derive(Deserialize)]
pub struct MergeForm {
    other_id: String,
}

/// Merge another unpaid order into this one (e.g. two bills at the same table). Items move; the other order is closed.
pub async fn merge(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<MergeForm>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    if id == f.other_id {
        return Err(AppError::Invalid("Pilih pesanan lain.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let open = |s: &str| matches!(s, "pending" | "confirmed" | "ready");
    let (a_status, a_num): (String, i64) = sqlx::query_as("SELECT status, number FROM orders WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let (b_status, b_num): (String, i64) = sqlx::query_as("SELECT status, number FROM orders WHERE id = ?")
        .bind(&f.other_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if !open(&a_status) || !open(&b_status) {
        return Err(AppError::Invalid(
            "Hanya pesanan yang belum dibayar yang bisa digabung.".into(),
        ));
    }
    for oid in [&id, &f.other_id] {
        if orders::paid_so_far(&mut tx, oid).await? > 0 {
            return Err(AppError::Invalid(
                "Pesanan dengan pembayaran sebagian tidak bisa digabung.".into(),
            ));
        }
    }
    sqlx::query("UPDATE order_items SET order_id = ? WHERE order_id = ?")
        .bind(&id)
        .bind(&f.other_id)
        .execute(&mut *tx)
        .await?;
    // The merged-away order keeps no items, so its totals fall to zero; stock is NOT restored (the items still exist).
    orders::recalc_public(&mut tx, &f.other_id, Some(0)).await?;
    let why = format!("Digabung ke #{a_num}");
    sqlx::query("UPDATE orders SET status = 'cancelled', cancel_reason = ? WHERE id = ?")
        .bind(&why)
        .bind(&f.other_id)
        .execute(&mut *tx)
        .await?;
    orders::recalc_public(&mut tx, &id, None).await?;
    let mut e = Entry::new(Some(&staff.id), "order.merged", "order", Some(&id));
    let detail = format!("#{b_num} -> #{a_num}");
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    orders::notify(&st);
    orders::render_detail(
        &st,
        &staff,
        &id,
        None,
        Some(format!("Pesanan #{b_num} digabung ke pesanan ini.")),
    )
    .await
}

// ---- API keys (FR-62) --------------------------------------------------------------------------

async fn keys_page(
    st: &AppState,
    staff: &Staff,
    new_key: Option<String>,
    error: Option<String>,
) -> AppResult<Response> {
    let tz = st.cfg.tz_offset_hours;
    let rows: Vec<(String, String, i64, Option<i64>, i64)> =
        sqlx::query_as("SELECT id, name, created_at, last_used, revoked FROM api_keys ORDER BY created_at DESC")
            .fetch_all(&st.db)
            .await?;
    let fmt = |t: i64| format!("{} {}", clock::day(t, tz), clock::hm(t, tz));
    Ok(render(ApiKeysPage {
        ctx: Ctx::staff(st, staff, "api").await?,
        error,
        new_key,
        base: st.cfg.public_url.trim_end_matches('/').to_string(),
        keys: rows
            .into_iter()
            .map(|(id, name, c, l, r)| (id, name, fmt(c), l.map(fmt).unwrap_or_else(|| "—".into()), r == 1))
            .collect(),
    })?
    .into_response())
}

pub async fn keys_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    staff.require(&[Role::Owner])?;
    keys_page(&st, &staff, None, None).await
}

#[derive(Deserialize)]
pub struct KeyNew {
    name: String,
}

pub async fn keys_add(State(st): State<AppState>, staff: Staff, Form(f): Form<KeyNew>) -> AppResult<Response> {
    staff.require(&[Role::Owner])?;
    let name = clean(&f.name, 40);
    if name.is_empty() {
        return keys_page(&st, &staff, None, Some("Nama kunci wajib diisi.".into())).await;
    }
    let token = format!("cuppa_{}", rand_id::hex_token(24));
    let id = rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("INSERT INTO api_keys (id, name, token_hash, created_by, created_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(&name)
        .bind(auth::hash_token(&token))
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "api_key.created", "api_key", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    keys_page(&st, &staff, Some(token), None).await
}

pub async fn keys_revoke(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner])?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE api_keys SET revoked = 1 WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "api_key.revoked", "api_key", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/api"))
}

fn api_error(code: axum::http::StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// Bearer auth for the read-only API. Keys are looked up by hash; each key is rate limited. Errors are JSON, not HTML.
async fn api_auth(st: &AppState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    use axum::http::StatusCode;
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Err(Box::new(api_error(StatusCode::UNAUTHORIZED, "missing bearer token")));
    };
    let h = auth::hash_token(token.trim());
    if !st.limiter.allow(&format!("api:{h}"), 120, Duration::from_secs(60)) {
        return Err(Box::new(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        )));
    }
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM api_keys WHERE token_hash = ? AND revoked = 0")
        .bind(&h)
        .fetch_optional(&st.db)
        .await
        .map_err(|_| Box::new(api_error(StatusCode::INTERNAL_SERVER_ERROR, "error")))?;
    let Some(id) = id else {
        return Err(Box::new(api_error(StatusCode::FORBIDDEN, "invalid or revoked key")));
    };
    let _ = sqlx::query("UPDATE api_keys SET last_used = ? WHERE id = ?")
        .bind(clock::now())
        .bind(&id)
        .execute(&st.db)
        .await;
    Ok(())
}

#[derive(Deserialize)]
pub struct ApiRange {
    from: Option<String>,
    to: Option<String>,
}

fn valid_day(s: &str) -> bool {
    s.len() == 10
        && s.bytes().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// `GET /api/v1/sales?from=YYYY-MM-DD&to=YYYY-MM-DD` — paid sales per day, JSON, read-only.
pub async fn api_sales(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(r): Query<ApiRange>,
) -> AppResult<Response> {
    if let Err(r) = api_auth(&st, &headers).await {
        return Ok(*r);
    }
    let today = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let (from, to) = (r.from.unwrap_or_else(|| today.clone()), r.to.unwrap_or(today));
    if !valid_day(&from) || !valid_day(&to) || from > to {
        return Ok(api_error(axum::http::StatusCode::BAD_REQUEST, "invalid date range"));
    }
    let rows: Vec<(String, i64, i64, i64, i64, i64)> = sqlx::query_as("SELECT day, COUNT(*), SUM(subtotal), SUM(discount), SUM(tax), SUM(total) FROM orders WHERE status = 'paid' AND day BETWEEN ? AND ? GROUP BY day ORDER BY day").bind(&from).bind(&to).fetch_all(&st.db).await?;
    let days: Vec<_> = rows.into_iter().map(|(d, n, sub, disc, tax, total)| json!({"day": d, "orders": n, "subtotal": sub, "discount": disc, "tax": tax, "total": total, "currency": "IDR"})).collect();
    Ok(Json(json!({ "from": from, "to": to, "days": days })).into_response())
}

/// `GET /api/v1/menu` — active menu with prices.
pub async fn api_menu(State(st): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Err(r) = api_auth(&st, &headers).await {
        return Ok(*r);
    }
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as("SELECT m.name, c.name, m.price, m.available FROM menu_items m JOIN categories c ON c.id = m.category_id WHERE m.active = 1 AND c.active = 1 ORDER BY c.name, m.name").fetch_all(&st.db).await?;
    let items: Vec<_> = rows
        .into_iter()
        .map(|(n, c, p, a)| json!({"name": n, "category": c, "price": p, "available": a == 1}))
        .collect();
    Ok(Json(json!({ "items": items, "currency": "IDR" })).into_response())
}

// ---- Ledger (FR-65) ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LedgerRange {
    from: Option<String>,
    to: Option<String>,
}

pub async fn ledger(
    State(st): State<AppState>,
    staff: Staff,
    Query(r): Query<LedgerRange>,
) -> AppResult<axum::response::Html<String>> {
    staff.need(auth::Perm::Reports)?;
    let today = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let from = r.from.filter(|s| valid_day(s)).unwrap_or_else(|| today.clone());
    let to = r.to.filter(|s| valid_day(s)).unwrap_or(today);
    let sum = |sql: &'static str| {
        let (st, from, to) = (st.clone(), from.clone(), to.clone());
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(from)
                .bind(to)
                .fetch_one(&st.db)
                .await
        }
    };
    let cash = sum("SELECT COALESCE(SUM(p.amount),0) FROM payments p JOIN orders o ON o.id = p.order_id WHERE o.status='paid' AND p.method='cash' AND o.day BETWEEN ? AND ?").await?;
    let bank = sum("SELECT COALESCE(SUM(p.amount),0) FROM payments p JOIN orders o ON o.id = p.order_id WHERE o.status='paid' AND p.method!='cash' AND o.day BETWEEN ? AND ?").await?;
    let sales =
        sum("SELECT COALESCE(SUM(subtotal - discount),0) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?")
            .await?;
    let service =
        sum("SELECT COALESCE(SUM(service),0) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?").await?;
    let tax = sum("SELECT COALESCE(SUM(tax),0) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?").await?;
    let rounding =
        sum("SELECT COALESCE(SUM(rounding),0) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?").await?;
    let refunds = sum("SELECT COALESCE(SUM(r.amount),0) FROM refunds r JOIN orders o ON o.id = r.order_id WHERE o.day BETWEEN ? AND ?").await?;
    let expenses = sum("SELECT COALESCE(SUM(amount),0) FROM expenses WHERE day BETWEEN ? AND ?").await?;
    let mut lines: Vec<(String, i64, i64)> = vec![
        ("Kas".into(), cash, 0),
        ("Bank / e-wallet".into(), bank, 0),
        ("Penjualan".into(), 0, sales),
        ("Pendapatan service".into(), 0, service),
        ("Utang pajak daerah".into(), 0, tax),
    ];
    if rounding > 0 {
        lines.push(("Pembulatan".into(), 0, rounding));
    } else if rounding < 0 {
        lines.push(("Pembulatan".into(), -rounding, 0));
    }
    // Refunds reduce sales and the asset that was paid back; operating expenses are shown as a memo (paid from cash/bank by the owner).
    lines.push(("Retur penjualan".into(), refunds, 0));
    lines.push(("Kas / bank (refund)".into(), 0, refunds));
    let (dr, cr): (i64, i64) = lines.iter().fold((0, 0), |a, l| (a.0 + l.1, a.1 + l.2));
    render(LedgerPage {
        ctx: Ctx::staff(&st, &staff, "ledger").await?,
        from,
        to,
        lines: lines
            .into_iter()
            .map(|(n, d, c)| {
                (
                    n,
                    if d > 0 { rupiah(d) } else { "—".into() },
                    if c > 0 { rupiah(c) } else { "—".into() },
                )
            })
            .collect(),
        debit: rupiah(dr),
        credit: rupiah(cr),
        balanced: dr == cr,
        expenses: rupiah(expenses),
    })
}
