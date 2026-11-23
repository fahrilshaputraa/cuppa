// SPDX-License-Identifier: AGPL-3.0-or-later
//! Receipts: printable HTML (any browser/printer) and ESC/POS to a LAN printer.

use crate::{
    AppState,
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
    escpos::{self, Receipt, ReceiptLine},
    handlers::load_settings,
    money::rupiah,
    views::*,
};
use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};

pub fn method_label(m: &str) -> &str {
    match m {
        "cash" => "Tunai",
        "qris" => "QRIS",
        "card" => "Kartu",
        _ => "Transfer",
    }
}

pub struct Data {
    pub outlet: String,
    pub number: i64,
    pub where_label: String,
    pub when: String,
    pub lines: Vec<ReceiptLine>,
    pub subtotal: i64,
    pub service: i64,
    pub tax: i64,
    pub total: i64,
    pub discount: i64,
    pub rounding: i64,
    pub payments: Vec<(String, i64, Option<i64>)>,
    pub token: Option<String>,
    pub footer: String,
    pub cols: usize,
    pub printer: Option<String>,
    pub drawer: bool,
}

pub async fn load(st: &AppState, id: &str) -> AppResult<Data> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    let o: (i64, i64, i64, i64, i64, i64, Option<String>, i64, i64) = sqlx::query_as(
        "SELECT o.number, o.subtotal, o.service, o.tax, o.total, COALESCE(o.paid_at, o.created_at), t.label, o.discount, o.rounding \
         FROM orders o LEFT JOIN tables t ON t.id = o.table_id WHERE o.id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(AppError::NotFound)?;
    let lines: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT qty, name, unit_price * qty FROM order_items WHERE order_id = ?")
            .bind(id)
            .fetch_all(&mut *c)
            .await?;
    let pays: Vec<(String, i64, Option<i64>)> =
        sqlx::query_as("SELECT method, amount, tendered FROM payments WHERE order_id = ? ORDER BY created_at")
            .bind(id)
            .fetch_all(&mut *c)
            .await?;
    let token: Option<String> = sqlx::query_scalar("SELECT receipt_token FROM orders WHERE id = ?")
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
    let tz = st.cfg.tz_offset_hours;
    Ok(Data {
        outlet: s.outlet_name,
        number: o.0,
        where_label: o.6.map(|l| format!("Meja {l}")).unwrap_or_else(|| "Antre".into()),
        when: format!("{} {}", clock::day(o.5, tz), clock::hm(o.5, tz)),
        lines: lines
            .into_iter()
            .map(|(qty, name, amount)| ReceiptLine { qty, name, amount })
            .collect(),
        subtotal: o.1,
        service: o.2,
        tax: o.3,
        total: o.4,
        discount: o.7,
        rounding: o.8,
        payments: pays
            .into_iter()
            .map(|(m, a, t)| (method_label(&m).to_string(), a, t))
            .collect(),
        token,
        footer: s.receipt_footer,
        cols: s.paper_cols as usize,
        printer: s.printer_host,
        drawer: s.open_drawer_on_cash == 1,
    })
}

fn bytes(d: &Data) -> Vec<u8> {
    escpos::receipt_bytes(&Receipt {
        outlet: &d.outlet,
        number: d.number,
        where_label: &d.where_label,
        when: &d.when,
        lines: &d.lines,
        subtotal: d.subtotal,
        service: d.service,
        tax: d.tax,
        total: d.total,
        discount: d.discount,
        rounding: d.rounding,
        payments: &d.payments,
        footer: &d.footer,
        cols: d.cols,
        open_drawer: d.drawer && d.payments.iter().any(|p| p.0 == "Tunai"),
    })
}

/// Best effort: a dead printer must never block or fail a sale.
pub fn auto_print(st: AppState, id: String) {
    tokio::spawn(async move {
        let Ok(d) = load(&st, &id).await else { return };
        let Some(addr) = d.printer.as_deref().and_then(|p| escpos::parse_printer(p).ok()) else {
            return;
        };
        if let Err(e) = escpos::send(addr, &bytes(&d)).await {
            tracing::warn!(error = ?e, "auto print failed");
        }
    });
}

/// Kitchen ticket to the kitchen/bar printer when "Sampai dapur" is on. Best effort.
pub fn print_ticket(st: AppState, id: String) {
    tokio::spawn(async move {
        let Ok(mut c) = st.db.acquire().await else { return };
        let Ok(s) = load_settings(&mut c).await else { return };
        if s.kitchen_mode != 1 {
            return;
        }
        let row: Option<(i64, i64, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT o.number, o.created_at, t.label, o.note FROM orders o LEFT JOIN tables t ON t.id = o.table_id WHERE o.id = ?").bind(&id).fetch_optional(&mut *c).await.ok().flatten();
        let Some((number, at, label, note)) = row else { return };
        // Each line goes to its station's printer; unmapped categories use the main kitchen printer.
        let lines: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT i.qty, i.name, s.printer_host FROM order_items i JOIN menu_items m ON m.id = i.menu_id JOIN categories c ON c.id = m.category_id \
             LEFT JOIN stations s ON s.id = c.station_id WHERE i.order_id = ? AND i.voided = 0",
        )
        .bind(&id)
        .fetch_all(&mut *c)
        .await
        .unwrap_or_default();
        let mut by_printer: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for (q, n, host) in lines {
            if let Some(h) = host.or_else(|| s.kitchen_printer_host.clone()) {
                by_printer.entry(h).or_default().push(format!("{q}x {n}"));
            }
        }
        let where_label = label.map(|l| format!("Meja {l}")).unwrap_or_else(|| "Antre".into());
        let when = clock::hm(at, st.cfg.tz_offset_hours);
        for (host, lines) in by_printer {
            let Ok(addr) = escpos::parse_printer(&host) else {
                continue;
            };
            let data = escpos::ticket_bytes(number, &where_label, &when, &lines, note.as_deref());
            if let Err(e) = escpos::send(addr, &data).await {
                tracing::warn!(error = ?e, "kitchen ticket print failed");
            }
        }
    });
}

/// Open the cash drawer without a sale. Best effort.
pub fn pulse_drawer(st: AppState) {
    tokio::spawn(async move {
        let Ok(mut c) = st.db.acquire().await else { return };
        let Ok(s) = load_settings(&mut c).await else { return };
        if let Some(addr) = s.printer_host.as_deref().and_then(|p| escpos::parse_printer(p).ok()) {
            let _ = escpos::send(addr, &escpos::drawer_pulse()).await;
        }
    });
}

pub async fn print(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let d = load(&st, &id).await?;
    let addr = d
        .printer
        .as_deref()
        .ok_or_else(|| AppError::Invalid("Printer belum diatur di Pengaturan.".into()))?;
    let addr = escpos::parse_printer(addr).map_err(AppError::Invalid)?;
    let ok = escpos::send(addr, &bytes(&d)).await.is_ok();
    Ok(Redirect::to(&format!("/orders/{id}/receipt?sent={}", ok as u8)))
}

#[derive(Template)]
#[template(path = "receipt.html")]
pub struct ReceiptPage {
    pub ctx: Ctx,
    pub id: String,
    pub sent: Option<bool>,
    pub has_printer: bool,
    pub share_url: Option<String>,
    pub wa_url: Option<String>,
    pub d: ReceiptView,
}

#[derive(Template)]
#[template(path = "public_receipt.html")]
pub struct PublicReceiptPage {
    pub ctx: Ctx,
    pub d: ReceiptView,
}

pub struct ReceiptView {
    pub outlet: String,
    pub number: i64,
    pub where_label: String,
    pub when: String,
    pub lines: Vec<(i64, String, String)>,
    pub subtotal: String,
    pub discount: Option<String>,
    pub service: String,
    pub tax: String,
    pub rounding: Option<String>,
    pub total: String,
    /// (label, amount shown, change)
    pub pays: Vec<(String, String, Option<String>)>,
    pub footer: String,
}

fn view(d: Data) -> ReceiptView {
    ReceiptView {
        outlet: d.outlet,
        number: d.number,
        where_label: d.where_label,
        when: d.when,
        lines: d.lines.into_iter().map(|l| (l.qty, l.name, rupiah(l.amount))).collect(),
        subtotal: rupiah(d.subtotal),
        discount: (d.discount > 0).then(|| format!("−{}", rupiah(d.discount))),
        service: rupiah(d.service),
        tax: rupiah(d.tax),
        rounding: (d.rounding != 0).then(|| rupiah(d.rounding)),
        total: rupiah(d.total),
        pays: d
            .payments
            .iter()
            .map(|(m, a, t)| {
                (
                    m.clone(),
                    rupiah(t.unwrap_or(*a)),
                    t.filter(|t| t > a).map(|t| rupiah(t - a)),
                )
            })
            .collect(),
        footer: d.footer,
    }
}

pub async fn page(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let d = load(&st, &id).await?;
    let share_url = d
        .token
        .as_ref()
        .map(|t| format!("{}/r/{}", st.cfg.public_url.trim_end_matches('/'), t));
    let wa_url = share_url.as_ref().map(|u| {
        format!(
            "https://wa.me/?text={}",
            pct_encode(&format!("Struk {}: {}", d.outlet, u))
        )
    });
    let has_printer = d.printer.is_some();
    let sent = q.get("sent").map(|v| v == "1");
    let page = ReceiptPage {
        ctx: Ctx::staff(&st, &staff, "cashier").await?,
        id,
        sent,
        has_printer,
        share_url,
        wa_url,
        d: view(d),
    };
    Ok(Html(page.render().map_err(|e| anyhow::anyhow!("template: {e}"))?).into_response())
}

/// Digital receipt (FR-52): unguessable 128-bit link, read-only, no staff data.
pub async fn public(State(st): State<AppState>, Path(token): Path<String>) -> AppResult<Response> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::NotFound);
    }
    let id: String =
        sqlx::query_scalar("SELECT id FROM orders WHERE receipt_token = ? AND status IN ('paid','voided')")
            .bind(&token)
            .fetch_optional(&st.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let d = load(&st, &id).await?;
    let page = PublicReceiptPage {
        ctx: Ctx::guest(&st).await?,
        d: view(d),
    };
    Ok(Html(page.render().map_err(|e| anyhow::anyhow!("template: {e}"))?).into_response())
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
