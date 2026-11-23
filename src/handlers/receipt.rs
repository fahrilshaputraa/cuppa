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
    pub method: Option<String>,
    pub tendered: Option<i64>,
    pub footer: String,
    pub cols: usize,
    pub printer: Option<String>,
    pub drawer: bool,
}

pub async fn load(st: &AppState, id: &str) -> AppResult<Data> {
    let mut c = st.db.acquire().await?;
    let s = load_settings(&mut c).await?;
    let o: (i64, i64, i64, i64, i64, i64, Option<String>) = sqlx::query_as(
        "SELECT o.number, o.subtotal, o.service, o.tax, o.total, COALESCE(o.paid_at, o.created_at), t.label \
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
    let pay: Option<(String, Option<i64>)> = sqlx::query_as("SELECT method, tendered FROM payments WHERE order_id = ?")
        .bind(id)
        .fetch_optional(&mut *c)
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
        method: pay.as_ref().map(|p| p.0.clone()),
        tendered: pay.and_then(|p| p.1),
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
        method: d.method.as_deref(),
        tendered: d.tendered,
        footer: &d.footer,
        cols: d.cols,
        open_drawer: d.drawer && d.method.as_deref() == Some("cash"),
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
        let Some(addr) = s
            .kitchen_printer_host
            .as_deref()
            .and_then(|p| escpos::parse_printer(p).ok())
        else {
            return;
        };
        let row: Option<(i64, i64, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT o.number, o.created_at, t.label, o.note FROM orders o LEFT JOIN tables t ON t.id = o.table_id WHERE o.id = ?").bind(&id).fetch_optional(&mut *c).await.ok().flatten();
        let Some((number, at, label, note)) = row else { return };
        let lines: Vec<(i64, String)> =
            sqlx::query_as("SELECT qty, name FROM order_items WHERE order_id = ? AND voided = 0")
                .bind(&id)
                .fetch_all(&mut *c)
                .await
                .unwrap_or_default();
        let lines: Vec<String> = lines.into_iter().map(|(q, n)| format!("{q}x {n}")).collect();
        let data = escpos::ticket_bytes(
            number,
            &label.map(|l| format!("Meja {l}")).unwrap_or_else(|| "Antre".into()),
            &clock::hm(at, st.cfg.tz_offset_hours),
            &lines,
            note.as_deref(),
        );
        if let Err(e) = escpos::send(addr, &data).await {
            tracing::warn!(error = ?e, "kitchen ticket print failed");
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
    pub d: ReceiptView,
}

pub struct ReceiptView {
    pub outlet: String,
    pub number: i64,
    pub where_label: String,
    pub when: String,
    pub lines: Vec<(i64, String, String)>,
    pub subtotal: String,
    pub service: String,
    pub tax: String,
    pub total: String,
    pub method: Option<String>,
    pub tendered: Option<String>,
    pub change: Option<String>,
    pub footer: String,
}

pub async fn page(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let d = load(&st, &id).await?;
    let change = d
        .tendered
        .filter(|t| *t > d.total && d.method.as_deref() == Some("cash"))
        .map(|t| rupiah(t - d.total));
    let method_label = d.method.as_deref().map(|m| {
        match m {
            "cash" => "Tunai",
            "qris" => "QRIS",
            "card" => "Kartu",
            _ => "Transfer",
        }
        .to_string()
    });
    let view = ReceiptView {
        outlet: d.outlet,
        number: d.number,
        where_label: d.where_label,
        when: d.when,
        lines: d.lines.into_iter().map(|l| (l.qty, l.name, rupiah(l.amount))).collect(),
        subtotal: rupiah(d.subtotal),
        service: rupiah(d.service),
        tax: rupiah(d.tax),
        total: rupiah(d.total),
        method: method_label,
        tendered: d.tendered.map(rupiah),
        change,
        footer: d.footer,
    };
    let sent = q.get("sent").map(|v| v == "1");
    let page = ReceiptPage {
        ctx: Ctx::staff(&st, &staff, "cashier").await?,
        id,
        sent,
        has_printer: d.printer.is_some(),
        d: view,
    };
    Ok(Html(page.render().map_err(|e| anyhow::anyhow!("template: {e}"))?).into_response())
}
