// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owner reports (sales, menu mix, payment methods, cashiers, voids, shift differences) + CSV export.

use crate::{
    AppState,
    auth::{Perm, Staff},
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
    pub cogs: String,
    pub gross: String,
    pub expenses: String,
    pub net: String,
    pub has_cost: bool,
    pub hours: Vec<(String, i64)>,
    pub max_hour: i64,
    pub margins: Vec<(String, String, String, String)>,
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
    staff.need(Perm::Reports)?;
    let (from, to) = resolve(&st, r)?;
    let (sales, orders): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(total),0), COUNT(*) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?",
    )
    .bind(&from)
    .bind(&to)
    .fetch_one(&st.db)
    .await?;
    let refunds: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(r.amount),0) FROM refunds r JOIN orders o ON o.id = r.order_id WHERE o.day BETWEEN ? AND ?").bind(&from).bind(&to).fetch_one(&st.db).await?;
    let sales = sales - refunds;
    let cogs: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(i.unit_cost * i.qty),0) FROM order_items i JOIN orders o ON o.id = i.order_id WHERE o.status='paid' AND i.voided = 0 AND o.day BETWEEN ? AND ?").bind(&from).bind(&to).fetch_one(&st.db).await?;
    let revenue_ex_tax: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total - tax),0) FROM orders WHERE status='paid' AND day BETWEEN ? AND ?",
    )
    .bind(&from)
    .bind(&to)
    .fetch_one(&st.db)
    .await?;
    let expenses: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM expenses WHERE day BETWEEN ? AND ?")
        .bind(&from)
        .bind(&to)
        .fetch_one(&st.db)
        .await?;
    // Busiest hours (outlet-local), FR-48.
    let tz = st.cfg.tz_offset_hours;
    let hour_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT strftime('%H', created_at + ? * 3600, 'unixepoch') AS h, COUNT(*) FROM orders WHERE status = 'paid' AND day BETWEEN ? AND ? GROUP BY h ORDER BY h",
    )
    .bind(tz as i64)
    .bind(&from)
    .bind(&to)
    .fetch_all(&st.db)
    .await?;
    let hours: Vec<(String, i64)> = (6..24)
        .map(|h| {
            (
                format!("{h:02}"),
                hour_rows
                    .iter()
                    .find(|r| r.0.parse::<i64>().ok() == Some(h))
                    .map(|r| r.1)
                    .unwrap_or(0),
            )
        })
        .collect();
    let max_hour = hours.iter().map(|h| h.1).max().unwrap_or(1).max(1);
    // Margin per menu from the cost snapshot on each line.
    let margin_rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT i.name, SUM(i.qty * i.unit_price), SUM(i.qty * i.unit_cost) FROM order_items i JOIN orders o ON o.id = i.order_id \
         WHERE o.status = 'paid' AND i.voided = 0 AND o.day BETWEEN ? AND ? GROUP BY i.name HAVING SUM(i.qty * i.unit_cost) > 0 ORDER BY (SUM(i.qty * i.unit_price) - SUM(i.qty * i.unit_cost)) DESC LIMIT 15",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(&st.db)
    .await?;
    let margins = margin_rows
        .into_iter()
        .map(|(n, rev, cost)| {
            (
                n,
                rupiah(rev - cost),
                format!("{}%", (rev - cost) * 100 / rev.max(1)),
                rupiah(rev),
            )
        })
        .collect();
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
        cogs: rupiah(cogs),
        gross: rupiah(revenue_ex_tax - cogs),
        expenses: rupiah(expenses),
        net: rupiah(revenue_ex_tax - cogs - expenses),
        has_cost: cogs > 0,
        hours,
        max_hour,
        margins,
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
    staff.need(Perm::Reports)?;
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

/// Local-tax (PBJT/PB1) summary per day for the period (FR-65). Rates stay the owner's responsibility.
pub async fn tax_csv(State(st): State<AppState>, staff: Staff, Query(r): Query<Range>) -> AppResult<Response> {
    staff.need(Perm::Reports)?;
    let (from, to) = resolve(&st, r)?;
    let rows: Vec<(String, i64, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT day, COUNT(*), SUM(subtotal), SUM(discount), SUM(service), SUM(tax), SUM(rounding), SUM(total) FROM orders WHERE status = 'paid' AND day BETWEEN ? AND ? GROUP BY day ORDER BY day",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(&st.db)
    .await?;
    let mut out = String::from("tanggal,jumlah_transaksi,subtotal,diskon,service,pajak,pembulatan,total\n");
    let mut sum = (0, 0, 0, 0, 0, 0, 0);
    for (d, n, sub, disc, svc, tax, rnd, total) in rows {
        out.push_str(&format!(
            "{},{n},{sub},{disc},{svc},{tax},{rnd},{total}\n",
            csv_cell(&d)
        ));
        sum = (
            sum.0 + n,
            sum.1 + sub,
            sum.2 + disc,
            sum.3 + svc,
            sum.4 + tax,
            sum.5 + rnd,
            sum.6 + total,
        );
    }
    out.push_str(&format!(
        "TOTAL,{},{},{},{},{},{},{}\n",
        sum.0, sum.1, sum.2, sum.3, sum.4, sum.5, sum.6
    ));
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"pajak-{from}_{to}.csv\""),
            ),
        ],
        out,
    )
        .into_response())
}

/// Double-entry sales journal (FR-65): Dr cash/bank per payment; Cr sales, service and tax payable per order.
pub async fn journal_csv(State(st): State<AppState>, staff: Staff, Query(r): Query<Range>) -> AppResult<Response> {
    staff.need(Perm::Reports)?;
    let (from, to) = resolve(&st, r)?;
    let orders: Vec<(String, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, day, number, subtotal - discount, service, tax, rounding FROM (SELECT id, day, number, subtotal, discount, service, tax, rounding FROM orders WHERE status = 'paid' AND day BETWEEN ? AND ?) ORDER BY day, number",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(&st.db)
    .await
    .map(|v: Vec<(String, String, i64, i64, i64, i64, i64)>| v.into_iter().map(|(id, d, n, net, svc, tax, rnd)| (id, d.replace('-', "").parse::<i64>().unwrap_or(0), n, net, svc, tax, rnd)).collect())?;
    let mut out = String::from("tanggal,no_pesanan,akun,debit,kredit,keterangan\n");
    for (id, d, n, net, svc, tax, rnd) in orders {
        let day = format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100);
        let pays: Vec<(String, i64)> = sqlx::query_as("SELECT method, amount FROM payments WHERE order_id = ?")
            .bind(&id)
            .fetch_all(&st.db)
            .await?;
        for (m, a) in pays {
            let acct = if m == "cash" { "Kas" } else { "Bank/E-wallet" };
            out.push_str(&format!(
                "{day},{n},{},{a},0,{}\n",
                csv_cell(acct),
                csv_cell(&format!("Pembayaran {m}"))
            ));
        }
        out.push_str(&format!(
            "{day},{n},{},0,{net},{}\n",
            csv_cell("Penjualan"),
            csv_cell("Penjualan bersih diskon")
        ));
        if svc > 0 {
            out.push_str(&format!(
                "{day},{n},{},0,{svc},{}\n",
                csv_cell("Pendapatan service"),
                csv_cell("Service charge")
            ));
        }
        if tax > 0 {
            out.push_str(&format!(
                "{day},{n},{},0,{tax},{}\n",
                csv_cell("Utang pajak daerah"),
                csv_cell("PBJT/PB1")
            ));
        }
        if rnd != 0 {
            out.push_str(&format!(
                "{day},{n},{},{},{},{}\n",
                csv_cell("Pembulatan"),
                if rnd < 0 { -rnd } else { 0 },
                rnd.max(0),
                csv_cell("Pembulatan kas")
            ));
        }
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"jurnal-{from}_{to}.csv\""),
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
