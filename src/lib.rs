// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cuppa — self-hosted POS and QR ordering for small cafes.
#![allow(clippy::type_complexity)] // sqlx row tuples

pub mod audit;
pub mod auth;
pub mod backup;
pub mod clock;
pub mod config;
pub mod db;
pub mod error;
pub mod escpos;
pub mod handlers;
pub mod money;
pub mod rand_id;
pub mod security;
pub mod totp;
pub mod views;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    middleware,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures_util::Stream;
use handlers::{admin, cashier, customer, extras, more, orders, owner, public, receipt, reports, session};
use sqlx::SqlitePool;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tower_http::{compression::CompressionLayer, services::ServeDir, timeout::TimeoutLayer, trace::TraceLayer};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<config::Config>,
    pub events: broadcast::Sender<String>,
    pub limiter: Arc<security::RateLimiter>,
    /// One-time first-run code (printed to the server log). `None` once an owner exists.
    pub setup_code: Arc<Mutex<Option<String>>>,
}

impl AppState {
    pub async fn new(cfg: config::Config) -> anyhow::Result<Self> {
        let db = db::connect(&cfg.database_url).await?;
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&db).await?;
        let setup_code = (users == 0).then(|| rand_id::hex_token(8));
        Ok(Self {
            db,
            cfg: Arc::new(cfg),
            events: broadcast::channel(64).0,
            limiter: Arc::new(security::RateLimiter::default()),
            setup_code: Arc::new(Mutex::new(setup_code)),
        })
    }
}

async fn events(State(st): State<AppState>, _staff: auth::Staff) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(st.events.subscribe())
        .filter_map(|m| m.ok())
        .map(|m| Ok(Event::default().event(m).data("1")));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}

async fn health() -> &'static str {
    "ok"
}

pub fn router(st: AppState) -> Router {
    let public = Router::new()
        .route("/login", get(session::login_page).post(session::login))
        .route("/setup", get(session::setup_page).post(session::setup))
        .route("/m/{token}", get(customer::menu))
        .route("/m/{token}/orders", post(customer::submit))
        .route("/m/{token}/orders/{id}", get(customer::status))
        .route("/m/{token}/orders/{id}/status", get(customer::status_part))
        .route("/m/{token}/orders/{id}/rating", post(public::rate))
        .route("/m/{token}/bill", get(public::bill))
        .route("/m/{token}/call", post(public::call))
        .route("/r/{token}", get(receipt::public))
        .route("/display", get(public::display))
        .route("/uploads/{name}", get(more::serve_upload))
        .route("/m/{token}/orders/{id}/loyalty", post(more::loyalty_join))
        .route("/display/part", get(public::display_part))
        .layer(middleware::from_fn_with_state(st.clone(), security::throttle_public));

    let staff = Router::new()
        .route("/", get(session::root))
        .route("/logout", post(session::logout))
        .route("/events", get(events))
        .route("/owner", get(owner::home))
        .route("/owner/menu", get(owner::menu))
        .route("/owner/categories", post(owner::add_category))
        .route("/owner/items", post(owner::add_item))
        .route("/owner/items/{id}/available", post(owner::toggle_available))
        .route("/owner/items/{id}/active", post(owner::toggle_active))
        .route("/owner/tables", get(owner::tables).post(owner::add_table))
        .route("/owner/tables/{id}/rotate", post(owner::rotate))
        .route("/owner/tables/{id}/qr", get(owner::qr))
        .route("/owner/staff", get(owner::staff).post(owner::add_staff))
        .route("/owner/staff/{id}/active", post(owner::toggle_staff))
        .route("/owner/settings", get(owner::settings).post(owner::save_settings))
        .route("/owner/audit", get(owner::audit_view))
        .route("/lock", get(extras::lock))
        .route("/calls/{id}/done", post(public::call_done))
        .route("/orders/{id}/voucher", post(orders::voucher))
        .route("/owner/wizard", get(extras::wizard_get).post(extras::wizard_post))
        .route("/owner/devices", get(extras::devices_get).post(extras::devices_add))
        .route("/owner/devices/{id}/revoke", post(extras::devices_revoke))
        .route("/owner/items/{id}", post(extras::item_edit))
        .route("/owner/items/{id}/modifiers", get(extras::modifiers_get))
        .route("/owner/items/{id}/groups", post(extras::group_add))
        .route("/owner/groups/{id}/modifiers", post(extras::modifier_add))
        .route("/owner/modifiers/{id}/active", post(extras::modifier_toggle))
        .route("/owner/menu/import", post(extras::menu_import))
        .route("/owner/staff/{id}/perms", post(extras::perms_save))
        .route("/menu/{id}/available", post(extras::sold_out))
        .route("/cashier/history", get(extras::history))
        .route("/cashier/shift/cash", post(extras::cash_move))
        .route("/orders/{id}/discount", post(orders::discount))
        .route("/orders/{id}/items/{item}/void", post(orders::void_item))
        .route("/orders/{id}/move", post(orders::move_table))
        .route("/orders/{id}/refund", post(orders::refund))
        .route(
            "/owner/items/{id}/photo",
            post(more::photo_upload).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/owner/stations", get(more::stations_get).post(more::stations_add))
        .route("/owner/categories/{id}/station", post(more::category_station))
        .route(
            "/cashier/reservations",
            get(more::reservations_get).post(more::reservations_add),
        )
        .route("/reservations/{id}/status", post(more::reservation_status))
        .route("/owner/customers", get(more::customers_get))
        .route("/owner/customers/{id}/erase", post(more::customer_erase))
        .route("/owner/security", get(more::security_get))
        .route("/owner/security/enable", post(more::security_enable))
        .route("/owner/security/disable", post(more::security_disable))
        .route("/owner/vouchers", get(admin::vouchers_get).post(admin::vouchers_add))
        .route("/owner/vouchers/{id}/active", post(admin::vouchers_toggle))
        .route("/owner/expenses", get(admin::expenses_get).post(admin::expenses_add))
        .route("/owner/reports", get(reports::page))
        .route("/owner/reports.csv", get(reports::csv))
        .route("/owner/reports/tax.csv", get(reports::tax_csv))
        .route("/owner/reports/journal.csv", get(reports::journal_csv))
        .route("/owner/backup", post(owner::backup_download))
        .route("/owner/printer/test", post(owner::printer_test))
        .route("/orders/{id}/receipt", get(receipt::page))
        .route("/orders/{id}/print", post(receipt::print))
        .route("/cashier", get(cashier::page))
        .route("/cashier/queue", get(cashier::queue))
        .route("/cashier/new", get(cashier::new_get).post(cashier::new_post))
        .route("/cashier/shift", get(cashier::shift_get))
        .route("/cashier/shift/open", post(cashier::shift_open))
        .route("/cashier/shift/close", post(cashier::shift_close))
        .route("/kitchen", get(cashier::kitchen_page))
        .route("/kitchen/tickets", get(more::kitchen_tickets))
        .route("/orders/{id}", get(orders::detail))
        .route("/orders/{id}/confirm", post(orders::confirm))
        .route("/orders/{id}/reject", post(orders::reject))
        .route("/orders/{id}/ready", post(orders::ready))
        .route("/orders/{id}/pay", post(orders::pay))
        .route("/orders/{id}/void", post(orders::void));

    Router::new()
        .merge(public)
        .merge(staff)
        .route("/healthz", get(health))
        .nest_service("/static", ServeDir::new("static"))
        .layer(middleware::from_fn(security::csrf))
        .layer(middleware::from_fn_with_state(st.clone(), security::headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(st)
}

/// Auto-cancel QR orders nobody confirmed in time (anti-prank).
pub fn spawn_housekeeping(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let r = expire_pending(&st).await;
            match r {
                Ok(r) if r.rows_affected() > 0 => orders::notify(&st),
                Err(e) => tracing::warn!(error = ?e, "housekeeping failed"),
                _ => {}
            }
            if let Err(e) = backup::daily(&st).await {
                tracing::warn!(error = ?e, "daily backup failed");
            }
            // Drop expired sessions.
            let _ = sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
                .bind(clock::now())
                .execute(&st.db)
                .await;
        }
    });
}

/// Cancel unconfirmed QR orders past the timeout and give their stock back, atomically.
async fn expire_pending(st: &AppState) -> sqlx::Result<sqlx::sqlite::SqliteQueryResult> {
    let mut tx = db::begin_write(&st.db).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM orders WHERE status='pending' AND created_at < ? - (SELECT confirm_timeout_min FROM outlet_settings WHERE id=1) * 60",
    )
    .bind(clock::now())
    .fetch_all(&mut *tx)
    .await?;
    let mut total = None;
    for id in &ids {
        orders::restore_stock(&mut tx, id).await?;
        let r = sqlx::query("UPDATE orders SET status='cancelled', cancel_reason='Tidak dikonfirmasi tepat waktu' WHERE id = ? AND status='pending'").bind(id).execute(&mut *tx).await?;
        total = Some(r);
    }
    tx.commit().await?;
    match total {
        Some(r) => Ok(r),
        None => sqlx::query("SELECT 1 WHERE 0").execute(&st.db).await,
    }
}
