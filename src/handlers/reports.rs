// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owner reports (sales, menu mix, payment methods, cashiers, voids, shift differences) + CSV export.

use crate::{
    AppState,
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
    money::rupiah,
    views::*,
};
use askama::Template;
use axum::{
    extract::{Query, State},
    http::header,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Range {
    from: Option<String>,
    to: Option<String>,
}

fn valid_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn resolve(st: &AppState, r: Range) -> Result<(String, String), AppError> {
    let today = clock::day(clock::now(), st.cfg.tz_offset_hours);
    let from = r.from.filter(|s| !s.is_empty()).unwrap_or_else(|| today.clone());
    let to = r.to.filter(|s| !s.is_empty()).unwrap_or(today);
    if !valid_day(&from) || !valid_day(&to) || from > to {
        return Err(AppError::Invalid("Rentang tanggal tidak valid.".into()));
    }
    Ok((from, to))
}

pub struct Row2 {
    pub label: String,
    pub count: i64,
    pub amount: String,
    pub raw: i64,
}

#[derive(Template)]
#[template(path = "owner_reports.html")]
pub struct ReportPage {
    pub ctx: Ctx,
    pub from: String,
    pub to: String,
    pub sales: String,
    pub orders: i64,
    pub average: String,
    pub voids: i64,
    pub void_amount: String,
    pub days: Vec<Row2>,
    pub menu: Vec<Row2>,
    pub methods: Vec<Row2>,
    pub cashiers: Vec<Row2>,
    pub shifts: Vec<(String, String, String, String)>,
    pub max_day: i64,
    pub max_menu: i64,
}

type Agg = (String, i64, i64);

fn rows(v: Vec<Agg>, map: impl Fn(String) -> String) -> Vec<Row2> {
    v.into_iter()
        .map(|(l, c, a)| Row2 {
            label: map(l),
            count: c,
            amount: rupiah(a),
            raw: a,
        })
        .collect()
}

fn method_label(m: String) -> String {
    match m.as_str() {
        "cash" => "Tunai",
        "qris" => "QRIS",
        "card" => "Kartu EDC",
        _ => "Transfer",
    }
    .to_string()
}

pub async fn page(State(st): State<AppState>, staff: Staff, Query(r): Query<Range>) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner])?;
    let (from, to) = resolve(&st, r)?;
    let (sales, orders): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(total),0), COUNT(*) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?",
    )
    .bind(&from)
    .bind(&to)
    .fetch_one(&st.db)
    .await?;
    let (voids, void_amount): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(total),0) FROM orders WHERE status='voided' AND day BETWEEN ? AND ?",
    )
    .bind(&from)
    .bind(&to)
    .fetch_one(&st.db)
    .await?;
    let days: Vec<Agg> = sqlx::query_as("SELECT day, COUNT(*), SUM(total) FROM orders WHERE status='paid' AND day BETWEEN ? AND ? GROUP BY day ORDER BY day").bind(&from).bind(&to).fetch_all(&st.db).await?;
    let menu: Vec<Agg> = sqlx::query_as("SELECT i.name, SUM(i.qty), SUM(i.qty*i.unit_price) FROM order_items i JOIN orders o ON o.id=i.order_id WHERE o.status='paid' AND o.day BETWEEN ? AND ? GROUP BY i.name ORDER BY 3 DESC LIMIT 20").bind(&from).bind(&to).fetch_all(&st.db).await?;
    let methods: Vec<Agg> = sqlx::query_as("SELECT p.method, COUNT(*), SUM(p.amount) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.status='paid' AND o.day BETWEEN ? AND ? GROUP BY p.method ORDER BY 3 DESC").bind(&from).bind(&to).fetch_all(&st.db).await?;
    let cashiers: Vec<Agg> = sqlx::query_as("SELECT u.name, COUNT(*), SUM(p.amount) FROM payments p JOIN orders o ON o.id=p.order_id JOIN users u ON u.id=p.cashier_id WHERE o.status='paid' AND o.day BETWEEN ? AND ? GROUP BY u.name ORDER BY 3 DESC").bind(&from).bind(&to).fetch_all(&st.db).await?;
    let tz = st.cfg.tz_offset_hours;
    let shifts: Vec<(i64, String, i64)> = sqlx::query_as("SELECT s.opened_at, u.name, s.difference FROM shifts s JOIN users u ON u.id=s.opened_by WHERE s.closed_at IS NOT NULL AND s.difference IS NOT NULL ORDER BY s.opened_at DESC LIMIT 10").fetch_all(&st.db).await?;
    let (days, menu) = (rows(days, |l| l), rows(menu, |l| l));
    let page = ReportPage {
        ctx: Ctx::staff(&st, &staff, "reports").await?,
        average: rupiah(if orders > 0 { sales / orders } else { 0 }),
        sales: rupiah(sales),
        orders,
        voids,
        void_amount: rupiah(void_amount),
        max_day: days.iter().map(|d| d.raw).max().unwrap_or(1).max(1),
        max_menu: menu.iter().map(|d| d.raw).max().unwrap_or(1).max(1),
        days,
        menu,
        methods: rows(methods, method_label),
        cashiers: rows(cashiers, |l| l),
        shifts: shifts
            .into_iter()
            .map(|(at, who, d)| {
                (
                    format!("{} {}", clock::day(at, tz), clock::hm(at, tz)),
                    who,
                    rupiah(d),
                    if d == 0 {
                        "ok"
                    } else if d < 0 {
                        "danger"
                    } else {
                        "warn"
                    }
                    .to_string(),
                )
            })
            .collect(),
        from,
        to,
    };
    Ok(Html(page.render().map_err(|e| anyhow::anyhow!("template: {e}"))?))
}

/// Spreadsheet formula injection guard: cells starting with = + - @ are neutralised.
fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub async fn csv(State(st): State<AppState>, staff: Staff, Query(r): Query<Range>) -> AppResult<Response> {
    staff.require(&[Role::Owner])?;
    let (from, to) = resolve(&st, r)?;
    let rows: Vec<(String, i64, String, String, i64, i64, i64, i64, String)> = sqlx::query_as(
        "SELECT o.day, o.number, o.source, o.status, o.subtotal, o.service, o.tax, o.total, COALESCE(p.method,'') \
         FROM orders o LEFT JOIN payments p ON p.order_id=o.id WHERE o.day BETWEEN ? AND ? ORDER BY o.day, o.number",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(&st.db)
    .await?;
    let mut out = String::from("day,number,source,status,subtotal,service,tax,total,method\n");
    for (day, n, src, status, sub, svc, tax, total, m) in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_cell(&day),
            n,
            csv_cell(&src),
            csv_cell(&status),
            sub,
            svc,
            tax,
            total,
            csv_cell(&m)
        ));
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"cuppa-{from}_{to}.csv\""),
            ),
        ],
        out,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_neutralises_formulas() {
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
    #[test]
    fn day_validation() {
        assert!(valid_day("2026-11-23"));
        assert!(!valid_day("2026-1-23"));
        assert!(!valid_day("2026/11/23"));
    }
}
