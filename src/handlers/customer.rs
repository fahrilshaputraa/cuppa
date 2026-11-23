// SPDX-License-Identifier: AGPL-3.0-or-later
//! Public ordering via table QR. No login. The token is not a secret, so every guard is server-side:
//! active table, QR enabled, re-priced totals, per-table pending cap, per-IP + per-token rate limit,
//! and cashier confirmation before anything reaches the kitchen.

use crate::{
    AppState, clock,
    error::{AppError, AppResult},
    handlers::{
        clean, hours_text, is_open, load_settings,
        orders::{self, NewOrder, create_order, parse_lines},
    },
    money::rupiah,
    views::*,
};
use axum::{
    Form,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use std::{net::SocketAddr, time::Duration};

const MAX_PENDING_PER_TABLE: i64 = 3;

async fn table_by_token(st: &AppState, token: &str) -> AppResult<(String, String)> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::NotFound);
    }
    let settings_on: i64 = sqlx::query_scalar("SELECT qr_ordering FROM outlet_settings WHERE id = 1")
        .fetch_one(&st.db)
        .await?;
    if settings_on != 1 {
        return Err(AppError::NotFound);
    }
    sqlx::query_as("SELECT id, label FROM tables WHERE token = ? AND active = 1")
        .bind(token)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)
}

async fn open_state(st: &AppState) -> AppResult<(bool, String)> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    Ok((
        is_open(&s, &clock::hm(clock::now(), st.cfg.tz_offset_hours)),
        hours_text(&s),
    ))
}

/// Automatic busy mode (FR-48): when many orders are active, warn customers and add to the wait estimate.
async fn busy_message(st: &AppState) -> AppResult<Option<String>> {
    let (threshold, extra): (i64, i64) =
        sqlx::query_as("SELECT busy_threshold, busy_extra_min FROM outlet_settings WHERE id = 1")
            .fetch_one(&st.db)
            .await?;
    if threshold == 0 {
        return Ok(None);
    }
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE status IN ('pending','confirmed')")
        .fetch_one(&st.db)
        .await?;
    Ok((active >= threshold).then(|| format!("Sedang ramai. Pesanan mungkin lebih lama sekitar {extra} menit")))
}

async fn menu_page(st: &AppState, token: &str, error: Option<String>) -> AppResult<Response> {
    let (_, label) = table_by_token(st, token).await?;
    let (open, hours) = open_state(st).await?;
    let busy = busy_message(st).await?;
    Ok(render(CustomerMenu {
        busy,
        ctx: Ctx::guest(st).await?,
        can_sold_out: false,
        open,
        hours,
        error,
        token: token.into(),
        table_label: label,
        menu: super::load_menu(st).await?,
    })?
    .into_response())
}

pub async fn menu(State(st): State<AppState>, Path(token): Path<String>) -> AppResult<Response> {
    menu_page(&st, &token, None).await
}

pub async fn submit(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let ip = crate::security::client_ip(&st, &headers, peer);
    if !st.limiter.allow(&format!("order-ip:{ip}"), 6, Duration::from_secs(300))
        || !st
            .limiter
            .allow(&format!("order-tok:{token}"), 10, Duration::from_secs(300))
    {
        return Err(AppError::TooMany);
    }
    let (table_id, _) = table_by_token(&st, &token).await?;
    if !open_state(&st).await?.0 {
        return menu_page(
            &st,
            &token,
            Some("Kafe sedang tutup, pesanan belum bisa dikirim.".into()),
        )
        .await;
    }
    let note = fields
        .iter()
        .find(|(k, _)| k == "note")
        .map(|(_, v)| clean(v, 200))
        .filter(|v| !v.is_empty());
    let items = match parse_lines(&fields, 20) {
        Ok(v) => v,
        Err(AppError::Invalid(m)) => return menu_page(&st, &token, Some(m)).await,
        Err(e) => return Err(e),
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE table_id = ? AND status = 'pending'")
        .bind(&table_id)
        .fetch_one(&mut *tx)
        .await?;
    if pending >= MAX_PENDING_PER_TABLE {
        return menu_page(
            &st,
            &token,
            Some("Pesanan sebelumnya masih menunggu konfirmasi kasir.".into()),
        )
        .await;
    }
    let (id, _) = match create_order(
        &mut tx,
        &st,
        NewOrder {
            source: "qr",
            table_id: Some(&table_id),
            cashier_id: None,
            items,
            note,
            channel: "dine_in",
        },
    )
    .await
    {
        Ok(v) => v,
        Err(AppError::Invalid(m)) => return menu_page(&st, &token, Some(m)).await,
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    orders::notify(&st);
    Ok(Redirect::to(&format!("/m/{token}/orders/{id}")).into_response())
}

async fn owned_order(st: &AppState, token: &str, id: &str) -> AppResult<(String, i64, String, i64)> {
    let (table_id, _) = table_by_token(st, token).await?;
    sqlx::query_as("SELECT id, number, status, total FROM orders WHERE id = ? AND table_id = ?")
        .bind(id)
        .bind(table_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn status(State(st): State<AppState>, Path((token, id)): Path<(String, String)>) -> AppResult<Html<String>> {
    let (id, number, _, _) = owned_order(&st, &token, &id).await?;
    render(CustomerStatus {
        ctx: Ctx::guest(&st).await?,
        token,
        id,
        number,
    })
}

pub async fn status_part(State(st): State<AppState>, Path((token, id)): Path<(String, String)>) -> AppResult<Response> {
    let (id, _, status, total) = owned_order(&st, &token, &id).await?;
    let lines: Vec<(i64, String)> =
        sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ? AND voided = 0")
            .bind(&id)
            .fetch_all(&st.db)
            .await?;
    let (class, label, hint) = match status.as_str() {
        "pending" => (
            "warn",
            "Menunggu konfirmasi kasir",
            "Pesanan Anda sudah masuk. Kasir akan segera mengonfirmasi",
        ),
        "confirmed" => ("neutral", "Sedang disiapkan", "Pembayaran dilakukan di kasir"),
        "ready" => ("ok", "Siap", "Pesanan Anda siap. Silakan bayar di kasir"),
        "paid" => ("ok", "Selesai", "Terima kasih"),
        "voided" => ("danger", "Dibatalkan", "Pesanan dibatalkan"),
        _ => ("danger", "Ditolak", "Pesanan tidak dapat diproses. Silakan tanya kasir"),
    };
    let paid = status == "paid";
    let rated: Option<i64> = sqlx::query_scalar("SELECT stars FROM order_ratings WHERE order_id = ?")
        .bind(&id)
        .fetch_optional(&st.db)
        .await?;
    let receipt: Option<String> = if paid {
        sqlx::query_scalar::<_, Option<String>>("SELECT receipt_token FROM orders WHERE id = ?")
            .bind(&id)
            .fetch_one(&st.db)
            .await?
            .map(|t| format!("/r/{t}"))
    } else {
        None
    };
    let rate_items: Vec<(String, String)> = if paid && rated.is_none() {
        sqlx::query_as("SELECT DISTINCT i.menu_id, m.name FROM order_items i JOIN menu_items m ON m.id = i.menu_id WHERE i.order_id = ? AND i.voided = 0")
            .bind(&id)
            .fetch_all(&st.db)
            .await?
    } else {
        Vec::new()
    };
    let can_join: bool = paid
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM order_customers WHERE order_id = ?)")
            .bind(&id)
            .fetch_one(&st.db)
            .await?;
    let html = render(CustomerStatusPart {
        can_join,
        rate_items,
        can_rate: paid && rated.is_none(),
        rated,
        token,
        id,
        receipt,
        class,
        label,
        hint,
        lines: lines.into_iter().map(|(q, n)| format!("{q}× {n}")).collect(),
        total: rupiah(total),
    })?;
    // 286 tells htmx to stop polling: final states no longer change, and the rating form must not be reset.
    let code = if matches!(status.as_str(), "paid" | "voided" | "cancelled") {
        axum::http::StatusCode::from_u16(286).unwrap_or(axum::http::StatusCode::OK)
    } else {
        axum::http::StatusCode::OK
    };
    Ok((code, html).into_response())
}
