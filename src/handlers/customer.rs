// SPDX-License-Identifier: AGPL-3.0-or-later
//! Public ordering via table QR. No login. The token is not a secret, so every guard is server-side:
//! active table, QR enabled, re-priced totals, per-table pending cap, per-IP + per-token rate limit,
//! and cashier confirmation before anything reaches the kitchen.

use crate::{
    AppState,
    error::{AppError, AppResult},
    handlers::{
        clean,
        orders::{self, NewOrder, create_order},
        parse_qty,
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

async fn menu_page(st: &AppState, token: &str, error: Option<String>) -> AppResult<Response> {
    let (_, label) = table_by_token(st, token).await?;
    let rows: Vec<MenuRow> = sqlx::query_as(
        "SELECT m.id, m.name, m.price, m.available, c.name AS category FROM menu_items m JOIN categories c ON c.id = m.category_id \
         WHERE m.active = 1 AND c.active = 1 ORDER BY c.sort_order, c.name, m.name",
    ).fetch_all(&st.db).await?;
    Ok(render(CustomerMenu {
        ctx: Ctx::guest(st).await?,
        error,
        token: token.into(),
        table_label: label,
        menu: group_menu(rows),
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
    let note = fields
        .iter()
        .find(|(k, _)| k == "note")
        .map(|(_, v)| clean(v, 200))
        .filter(|v| !v.is_empty());
    let items = match parse_qty(fields, 20) {
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

pub async fn status_part(
    State(st): State<AppState>,
    Path((token, id)): Path<(String, String)>,
) -> AppResult<Html<String>> {
    let (id, _, status, total) = owned_order(&st, &token, &id).await?;
    let lines: Vec<(i64, String)> = sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ?")
        .bind(&id)
        .fetch_all(&st.db)
        .await?;
    let (class, label, hint) = match status.as_str() {
        "pending" => (
            "warn",
            "Menunggu konfirmasi kasir",
            "Pesanan Anda sudah masuk. Kasir akan segera mengonfirmasi.",
        ),
        "confirmed" => ("neutral", "Sedang disiapkan", "Pembayaran dilakukan di kasir."),
        "ready" => ("ok", "Siap", "Pesanan Anda siap. Silakan bayar di kasir."),
        "paid" => ("ok", "Selesai", "Terima kasih!"),
        "voided" => ("danger", "Dibatalkan", "Pesanan dibatalkan."),
        _ => (
            "danger",
            "Ditolak",
            "Pesanan tidak dapat diproses. Silakan tanya kasir.",
        ),
    };
    render(CustomerStatusPart {
        class,
        label,
        hint,
        lines: lines.into_iter().map(|(q, n)| format!("{q}× {n}")).collect(),
        total: rupiah(total),
    })
}
