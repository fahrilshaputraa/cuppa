// SPDX-License-Identifier: AGPL-3.0-or-later
//! Public, anonymous endpoints beyond ordering: queue display, ratings, table calls, table bill.
//! Nothing here exposes personal data; every write is rate-limited and bound to a valid table token.

use crate::{
    AppState,
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
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{net::SocketAddr, time::Duration};

// ---- Queue display (FR-62) ---------------------------------------------------------------------

pub async fn display(State(st): State<AppState>) -> AppResult<Html<String>> {
    render(DisplayPage {
        ctx: Ctx::guest(&st).await?,
    })
}

pub async fn display_part(State(st): State<AppState>) -> AppResult<Html<String>> {
    let day = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let working: Vec<i64> = sqlx::query_scalar(
        "SELECT number FROM orders WHERE day = ? AND status IN ('pending','confirmed') ORDER BY number",
    )
    .bind(&day)
    .fetch_all(&st.db)
    .await?;
    let ready: Vec<i64> = sqlx::query_scalar("SELECT number FROM orders WHERE day = ? AND status IN ('ready','paid') ORDER BY COALESCE(paid_at, created_at) DESC LIMIT 8").bind(&day).fetch_all(&st.db).await?;
    render(DisplayPart { working, ready })
}

// ---- Table token helper ------------------------------------------------------------------------

async fn table_by_token(st: &AppState, token: &str) -> AppResult<(String, String)> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::NotFound);
    }
    let on: i64 = sqlx::query_scalar("SELECT qr_ordering FROM outlet_settings WHERE id = 1")
        .fetch_one(&st.db)
        .await?;
    if on != 1 {
        return Err(AppError::NotFound);
    }
    sqlx::query_as("SELECT id, label FROM tables WHERE token = ? AND active = 1")
        .bind(token)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)
}

fn limit(st: &AppState, headers: &HeaderMap, peer: SocketAddr, token: &str, what: &str) -> AppResult<()> {
    let ip = crate::security::client_ip(st, headers, peer);
    if !st
        .limiter
        .allow(&format!("{what}-ip:{ip}"), 10, Duration::from_secs(300))
        || !st
            .limiter
            .allow(&format!("{what}-tok:{token}"), 12, Duration::from_secs(300))
    {
        return Err(AppError::TooMany);
    }
    Ok(())
}

// ---- Call waiter / ask for the bill (FR-53) ----------------------------------------------------

#[derive(Deserialize)]
pub struct CallForm {
    kind: String,
}

pub async fn call(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Form(f): Form<CallForm>,
) -> AppResult<Response> {
    limit(&st, &headers, peer, &token, "call")?;
    let (table_id, _) = table_by_token(&st, &token).await?;
    if !["waiter", "bill"].contains(&f.kind.as_str()) {
        return Err(AppError::Invalid("Jenis panggilan tidak valid.".into()));
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let open: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM table_calls WHERE table_id = ? AND kind = ? AND handled_at IS NULL)",
    )
    .bind(&table_id)
    .bind(&f.kind)
    .fetch_one(&mut *tx)
    .await?;
    if !open {
        sqlx::query("INSERT INTO table_calls (id, table_id, kind, created_at) VALUES (?,?,?,?)")
            .bind(rand_id::id())
            .bind(&table_id)
            .bind(&f.kind)
            .bind(clock::now())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    orders::notify(&st);
    Ok(Redirect::to(&format!("/m/{token}/bill?called={}", f.kind)).into_response())
}

pub async fn call_done(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    sqlx::query("UPDATE table_calls SET handled_at = ?, handled_by = ? WHERE id = ? AND handled_at IS NULL")
        .bind(clock::now())
        .bind(&staff.id)
        .bind(&id)
        .execute(&st.db)
        .await?;
    orders::notify(&st);
    Ok(Redirect::to("/cashier"))
}

// ---- Table bill: everything ordered at this table today (FR-07 / FR-53) ------------------------

pub async fn bill(
    State(st): State<AppState>,
    Path(token): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let (table_id, label) = table_by_token(&st, &token).await?;
    let day = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let rows: Vec<(String, i64, String, i64)> = sqlx::query_as("SELECT id, number, status, total FROM orders WHERE table_id = ? AND day = ? AND status IN ('pending','confirmed','ready','paid') ORDER BY number").bind(&table_id).bind(&day).fetch_all(&st.db).await?;
    let mut orders_v = Vec::new();
    let mut open_total = 0;
    for (id, number, status, total) in rows {
        let lines: Vec<(i64, String)> =
            sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ? AND voided = 0")
                .bind(&id)
                .fetch_all(&st.db)
                .await?;
        let (label_s, class) = orders::status_meta(&status);
        if status != "paid" {
            open_total += total;
        }
        orders_v.push(BillOrder {
            id,
            number,
            status_label: label_s,
            status_class: class,
            total: rupiah(total),
            lines: lines.into_iter().map(|(q, n)| format!("{q}× {n}")).collect(),
            paid: status == "paid",
        });
    }
    render(CustomerBill {
        ctx: Ctx::guest(&st).await?,
        token,
        table_label: label,
        orders: orders_v,
        open_total: rupiah(open_total),
        called: q.get("called").cloned(),
    })
}

// ---- Ratings (FR-47) ---------------------------------------------------------------------------

pub async fn rate(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((token, id)): Path<(String, String)>,
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    limit(&st, &headers, peer, &token, "rate")?;
    let (table_id, _) = table_by_token(&st, &token).await?;
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string());
    let stars: i64 = get("stars").and_then(|v| v.parse().ok()).unwrap_or(0);
    if !(1..=5).contains(&stars) {
        return Err(AppError::Invalid("Nilai harus 1–5.".into()));
    }
    let paid: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM orders WHERE id = ? AND table_id = ? AND status = 'paid')")
            .bind(&id)
            .bind(&table_id)
            .fetch_one(&st.db)
            .await?;
    if !paid {
        return Err(AppError::NotFound);
    }
    let comment = clean(&get("comment").unwrap_or_default(), 200);
    let mut tx = crate::db::begin_write(&st.db).await?;
    // One rating per order: INSERT OR IGNORE keeps the first one (and its per-menu stars).
    let first =
        sqlx::query("INSERT OR IGNORE INTO order_ratings (order_id, stars, comment, created_at) VALUES (?,?,?,?)")
            .bind(&id)
            .bind(stars)
            .bind(if comment.is_empty() { None } else { Some(comment) })
            .bind(clock::now())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
    if first {
        for (k, v) in &fields {
            let Some(menu_id) = k.strip_prefix("item_") else {
                continue;
            };
            let Ok(s) = v.trim().parse::<i64>() else { continue };
            if !(1..=5).contains(&s) {
                continue;
            }
            // Only items that are really on this order can be rated.
            sqlx::query("INSERT OR IGNORE INTO order_item_ratings (order_id, menu_id, stars) SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM order_items WHERE order_id = ? AND menu_id = ?)")
                .bind(&id)
                .bind(menu_id)
                .bind(s)
                .bind(&id)
                .bind(menu_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(Redirect::to(&format!("/m/{token}/orders/{id}")).into_response())
}
