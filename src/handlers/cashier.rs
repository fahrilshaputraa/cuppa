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
    let orders = orders::queue_orders(&st, &["pending", "confirmed", "ready"]).await?;
    let calls: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT c.id, t.label, c.kind, c.created_at FROM table_calls c JOIN tables t ON t.id = c.table_id WHERE c.handled_at IS NULL ORDER BY c.created_at",
    )
    .fetch_all(&st.db)
    .await?;
    let tz = st.cfg.tz_offset_hours;
    let calls: Vec<CallView> = calls
        .into_iter()
        .map(|(id, label, kind, at)| CallView {
            id,
            label: format!("Meja {label}"),
            kind_label: if kind == "bill" {
                "minta bill"
            } else {
                "memanggil pelayan"
            },
            time: clock::hm(at, tz),
        })
        .collect();
    let pending = orders.iter().filter(|o| o.status == "pending").count() + calls.len();
    render(CashierQueue { orders, calls, pending })
}

pub async fn kitchen_page(
    State(st): State<AppState>,
    staff: Staff,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Kitchen])?;
    if !Ctx::guest(&st).await?.kitchen {
        return Err(AppError::Invalid(
            "Mode dapur belum diaktifkan. Nyalakan di Pengaturan → Alur dapur.".into(),
        ));
    }
    let stations: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM stations ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    render(KitchenPage {
        ctx: Ctx::staff(&st, &staff, "kitchen").await?,
        stations,
        station_q: q
            .get("station")
            .filter(|v| v.len() <= 40 && v.bytes().all(|b| b.is_ascii_alphanumeric()))
            .cloned(),
    })
}

pub async fn kitchen_tickets(State(st): State<AppState>, staff: Staff) -> AppResult<Html<String>> {
    staff.require(&[Role::Owner, Role::Kitchen])?;
    render(KitchenTickets {
        orders: orders::queue_orders(&st, &["confirmed"]).await?,
    })
}

async fn new_page(st: &AppState, staff: &Staff, error: Option<String>) -> AppResult<Response> {
    let menu = super::load_menu(st).await?;
    let tables: Vec<TableRow> = sqlx::query_as("SELECT id, label, active FROM tables WHERE active = 1 ORDER BY label")
        .fetch_all(&st.db)
        .await?;
    Ok(render(CashierNew {
        ctx: Ctx::staff(st, staff, "new").await?,
        can_sold_out: staff.can(crate::auth::Perm::SoldOut),
        error,
        menu,
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
    let channel = fields
        .iter()
        .find(|(k, _)| k == "channel")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "dine_in".into());
    if !["dine_in", "takeaway", "gofood", "grabfood", "shopeefood"].contains(&channel.as_str()) {
        return new_page(&st, &staff, Some("Tipe pesanan tidak valid.".into())).await;
    }
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
            channel: &channel,
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
    let open_id: Option<String> = sqlx::query_scalar("SELECT id FROM shifts WHERE closed_at IS NULL")
        .fetch_optional(&st.db)
        .await?;
    let tz = st.cfg.tz_offset_hours;
    let movements = match &open_id {
        Some(id) => {
            let rows: Vec<(i64, String, i64, String, String)> = sqlx::query_as(
                "SELECT m.created_at, m.kind, m.amount, m.reason, u.name FROM cash_movements m JOIN users u ON u.id = m.user_id WHERE m.shift_id = ? ORDER BY m.created_at DESC",
            )
            .bind(id)
            .fetch_all(&st.db)
            .await?;
            rows.into_iter()
                .map(|(at, kind, amount, reason, who)| MoveView {
                    time: clock::hm(at, tz),
                    kind: match kind.as_str() {
                        "in" => "Kas masuk",
                        "out" => "Kas keluar",
                        _ => "Buka laci",
                    },
                    amount: if kind == "open_drawer" {
                        "—".into()
                    } else {
                        rupiah(amount)
                    },
                    reason,
                    who,
                })
                .collect()
        }
        None => Vec::new(),
    };
    Ok(render(ShiftPage {
        ctx: Ctx::staff(st, staff, "shift").await?,
        error,
        open: open_id.is_some(),
        report,
        movements,
        can_cash: staff.can(crate::auth::Perm::CashMove),
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
    let cash_in: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM cash_movements WHERE shift_id = ? AND kind = 'in'")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    let cash_out: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM cash_movements WHERE shift_id = ? AND kind = 'out'")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    let expected = opening + cash + cash_in - cash_out;
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
        cash_in: rupiah(cash_in),
        cash_out: rupiah(cash_out),
    };
    shift_page(&st, &staff, None, Some(report)).await
}
