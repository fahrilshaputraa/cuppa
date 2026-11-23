// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owner pages: vouchers (FR-49) and operating expenses (FR-45).

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    clock,
    error::AppResult,
    handlers::clean,
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

// ---- Vouchers ----------------------------------------------------------------------------------

async fn vouchers_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let rows: Vec<(String, String, String, i64, i64, Option<i64>, i64, i64)> = sqlx::query_as(
        "SELECT id, code, kind, value, min_subtotal, max_uses, used, active FROM vouchers ORDER BY active DESC, code",
    )
    .fetch_all(&st.db)
    .await?;
    let vouchers = rows
        .into_iter()
        .map(|(id, code, kind, value, min, max, used, active)| VoucherView {
            id,
            code,
            value: if kind == "pct" {
                format!("{value}%")
            } else {
                rupiah(value)
            },
            min: if min > 0 { rupiah(min) } else { "—".into() },
            uses: match max {
                Some(m) => format!("{used}/{m}"),
                None => used.to_string(),
            },
            active: active == 1,
        })
        .collect();
    Ok(render(VouchersPage {
        ctx: Ctx::staff(st, staff, "vouchers").await?,
        error,
        vouchers,
    })?
    .into_response())
}

pub async fn vouchers_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    vouchers_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct VoucherNew {
    code: String,
    kind: String,
    value: i64,
    min_subtotal: Option<i64>,
    max_uses: Option<String>,
}

pub async fn vouchers_add(State(st): State<AppState>, staff: Staff, Form(f): Form<VoucherNew>) -> AppResult<Response> {
    owner(&staff)?;
    let code = clean(&f.code, 20).to_uppercase();
    let max_uses: Option<i64> = f
        .max_uses
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok());
    let ok = code.len() >= 3
        && code.bytes().all(|b| b.is_ascii_alphanumeric())
        && ["pct", "rp"].contains(&f.kind.as_str())
        && f.value > 0
        && (f.kind != "pct" || f.value <= 100)
        && f.value <= 100_000_000;
    if !ok {
        return vouchers_page(
            &st,
            &staff,
            Some("Kode 3–20 huruf/angka, nilai > 0 (persen maksimal 100).".into()),
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    let r = sqlx::query("INSERT INTO vouchers (id, code, kind, value, min_subtotal, max_uses) VALUES (?,?,?,?,?,?)")
        .bind(&id)
        .bind(&code)
        .bind(&f.kind)
        .bind(f.value)
        .bind(f.min_subtotal.unwrap_or(0).max(0))
        .bind(max_uses)
        .execute(&mut *tx)
        .await;
    if r.is_err() {
        drop(tx);
        return vouchers_page(&st, &staff, Some("Kode sudah dipakai.".into())).await;
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "voucher.created", "voucher", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/vouchers").into_response())
}

pub async fn vouchers_toggle(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Redirect> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE vouchers SET active = 1 - active WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "voucher.toggled", "voucher", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/vouchers"))
}

// ---- Expenses ----------------------------------------------------------------------------------

async fn expenses_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let tz = st.cfg.tz_offset_hours;
    let month = clock::day(clock::now(), tz)[..7].to_string();
    let rows: Vec<(String, String, String, i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, day, category, amount, kind, note FROM expenses WHERE day LIKE ? ORDER BY day DESC, created_at DESC LIMIT 200",
    )
    .bind(format!("{month}%"))
    .fetch_all(&st.db)
    .await?;
    let total: i64 = rows.iter().map(|r| r.3).sum();
    let fixed: i64 = rows.iter().filter(|r| r.4 == "fixed").map(|r| r.3).sum();
    Ok(render(ExpensesPage {
        ctx: Ctx::staff(st, staff, "expenses").await?,
        error,
        today: clock::day(clock::now(), tz),
        month,
        total: rupiah(total),
        fixed: rupiah(fixed),
        variable: rupiah(total - fixed),
        rows: rows
            .into_iter()
            .map(|(_, day, category, amount, kind, note)| {
                (
                    day,
                    category,
                    rupiah(amount),
                    if kind == "fixed" { "Tetap" } else { "Variabel" }.to_string(),
                    note.unwrap_or_default(),
                )
            })
            .collect(),
    })?
    .into_response())
}

pub async fn expenses_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    expenses_page(&st, &staff, None).await
}

#[derive(Deserialize)]
pub struct ExpenseNew {
    day: String,
    category: String,
    amount: i64,
    kind: String,
    note: Option<String>,
}

pub async fn expenses_add(State(st): State<AppState>, staff: Staff, Form(f): Form<ExpenseNew>) -> AppResult<Response> {
    owner(&staff)?;
    let category = clean(&f.category, 40);
    let day_ok = f.day.len() == 10
        && f.day.bytes().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    if category.is_empty()
        || f.amount <= 0
        || f.amount > 1_000_000_000
        || !day_ok
        || !["fixed", "variable"].contains(&f.kind.as_str())
    {
        return expenses_page(
            &st,
            &staff,
            Some("Tanggal, kategori, dan nominal wajib diisi dengan benar.".into()),
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    sqlx::query(
        "INSERT INTO expenses (id, day, category, amount, kind, note, user_id, created_at) VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&f.day)
    .bind(&category)
    .bind(f.amount)
    .bind(&f.kind)
    .bind(f.note.as_deref().map(|n| clean(n, 120)).filter(|n| !n.is_empty()))
    .bind(&staff.id)
    .bind(clock::now())
    .execute(&mut *tx)
    .await?;
    let detail = format!("{category} {}", f.amount);
    let mut e = Entry::new(Some(&staff.id), "expense.created", "expense", Some(&id));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/expenses").into_response())
}
