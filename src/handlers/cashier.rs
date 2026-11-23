// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::orders::{self, NewOrder, create_order},
    money::rupiah,
    rand_id,
    views::*,
};
use axum::{
    Form,
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

pub async fn page(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    render(CashierPage {
        ctx: Ctx::staff(&st, &staff, "cashier").await?,
    })
}

pub async fn queue(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    render(CashierQueue {
        orders: orders::queue_orders(&st, &["pending", "confirmed", "ready"]).await?,
    })
}

pub async fn kitchen_page(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Kitchen])?;
    render(KitchenPage {
        ctx: Ctx::staff(&st, &staff, "kitchen").await?,
    })
}

pub async fn kitchen_tickets(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Kitchen])?;
    render(KitchenTickets {
        orders: orders::queue_orders(&st, &["confirmed"]).await?,
    })
}

async fn new_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let rows: Vec<MenuRow> = sqlx::query_as(
        "SELECT m.id, m.name, m.price, m.available, c.name AS category FROM menu_items m JOIN categories c ON c.id = m.category_id \
         WHERE m.active = 1 AND c.active = 1 ORDER BY c.sort_order, c.name, m.name",
    )
    .fetch_all(&st.db).await?;
    let tables: Vec<TableRow> = sqlx::query_as("SELECT id, label, active FROM tables WHERE active = 1 ORDER BY label")
        .fetch_all(&st.db)
        .await?;
    Ok(render(CashierNew {
        ctx: Ctx::staff(st, staff, "new").await?,
        error,
        menu: group_menu(rows),
        tables,
    })?
    .into_response())
}

pub async fn new_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    new_page(&st, &staff, None).await
}

pub async fn new_post(
    State(st): State<AppState>,
    staff: Staff,
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let (items, note, table) = match orders::parse_manual(fields).await {
        Ok(v) => v,
        Err(AppError::Invalid(m)) => return new_page(&st, &staff, Some(m)).await,
        Err(e) => return Err(e),
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    if let Some(t) = &table {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tables WHERE id = ? AND active = 1)")
            .bind(t)
            .fetch_one(&mut *tx)
            .await?;
        if !ok {
            return new_page(&st, &staff, Some("Meja tidak valid.".into())).await;
        }
    }
    let (id, _) = match create_order(
        &mut tx,
        &st,
        NewOrder {
            source: "cashier",
            table_id: table.as_deref(),
            cashier_id: Some(&staff.id),
            items,
            note,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(AppError::Invalid(m)) => return new_page(&st, &staff, Some(m)).await,
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    orders::notify(&st);
    Ok(Redirect::to(&format!("/orders/{id}")).into_response())
}

// ---- Shifts ----

async fn shift_page(
    st: &AppState,
    staff: &Staff,
    error: Option<String>,
    report: Option<ZReport>,
) -> AppResult<Response> {
    let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM shifts WHERE closed_at IS NULL)")
        .fetch_one(&st.db)
        .await?;
    Ok(render(ShiftPage {
        ctx: Ctx::staff(st, staff, "shift").await?,
        error,
        open,
        report,
    })?
    .into_response())
}

pub async fn shift_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    shift_page(&st, &staff, None, None).await
}

#[derive(Deserialize)]
pub struct OpenForm {
    opening: i64,
}

pub async fn shift_open(State(st): State<AppState>, staff: Staff, Form(f): Form<OpenForm>) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    if !(0..=1_000_000_000).contains(&f.opening) {
        return shift_page(&st, &staff, Some("Modal awal tidak valid.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    // The partial unique index guarantees a single open shift even under concurrent requests.
    let r = sqlx::query("INSERT INTO shifts (id, opened_by, opening_cash, opened_at) VALUES (?,?,?,?)")
        .bind(&id)
        .bind(&staff.id)
        .bind(f.opening)
        .bind(clock::now())
        .execute(&mut *tx)
        .await;
    if r.is_err() {
        return shift_page(&st, &staff, Some("Sudah ada shift yang terbuka.".into()), None).await;
    }
    audit::record(&mut tx, Entry::new(Some(&staff.id), "shift.opened", "shift", Some(&id))).await?;
    tx.commit().await?;
    Ok(Redirect::to("/cashier/shift").into_response())
}

#[derive(Deserialize)]
pub struct CloseForm {
    counted: i64,
}

/// Blind close: the cashier commits the physical count before the system figure is shown.
pub async fn shift_close(State(st): State<AppState>, staff: Staff, Form(f): Form<CloseForm>) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    if !(0..=1_000_000_000).contains(&f.counted) {
        return shift_page(&st, &staff, Some("Jumlah tidak valid.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let row: Option<(String, i64)> = sqlx::query_as("SELECT id, opening_cash FROM shifts WHERE closed_at IS NULL")
        .fetch_optional(&mut *tx)
        .await?;
    let (id, opening) = row.ok_or_else(|| AppError::Invalid("Tidak ada shift terbuka.".into()))?;
    let cash: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(p.amount),0) FROM payments p JOIN orders o ON o.id = p.order_id WHERE p.shift_id = ? AND p.method = 'cash' AND o.status = 'paid'")
        .bind(&id).fetch_one(&mut *tx).await?;
    let expected = opening + cash;
    let diff = f.counted - expected;
    sqlx::query("UPDATE shifts SET closed_at = ?, counted_cash = ?, expected_cash = ?, difference = ? WHERE id = ?")
        .bind(clock::now())
        .bind(f.counted)
        .bind(expected)
        .bind(diff)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let detail = format!("diff {diff}");
    let mut e = Entry::new(Some(&staff.id), "shift.closed", "shift", Some(&id));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    let report = ZReport {
        opening: rupiah(opening),
        cash_sales: rupiah(cash),
        expected: rupiah(expected),
        counted: rupiah(f.counted),
        difference: rupiah(diff),
    };
    shift_page(&st, &staff, None, Some(report)).await
}
