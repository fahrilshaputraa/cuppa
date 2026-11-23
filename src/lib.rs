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
pub mod views;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    middleware,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures_util::Stream;
use handlers::{cashier, customer, orders, owner, receipt, reports, session};
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
        .route("/owner/reports", get(reports::page))
        .route("/owner/reports.csv", get(reports::csv))
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
        .route("/kitchen/tickets", get(cashier::kitchen_tickets))
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
            let r = sqlx::query(
                "UPDATE orders SET status='cancelled', cancel_reason='Tidak dikonfirmasi tepat waktu' WHERE status='pending' \
                 AND created_at < ? - (SELECT confirm_timeout_min FROM outlet_settings WHERE id=1) * 60",
            )
            .bind(clock::now())
            .execute(&st.db)
            .await;
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
