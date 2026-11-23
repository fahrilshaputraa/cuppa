// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ingredients, recipes and stock movements (FR-44). Quantities are integers in a base unit; costs are rupiah x 1000.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
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
use sqlx::SqliteConnection;

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

fn cost_fmt(x1000: i64) -> String {
    // rupiah per base unit with up to 2 decimals (e.g. Rp0,12 per gram)
    let cents = x1000 / 10;
    format!("Rp{},{:02}", cents / 100, cents % 100)
}

/// Recipe cost of one portion of a menu item (rupiah). `None` when the item has no recipe.
pub async fn recipe_cost(conn: &mut SqliteConnection, menu_id: &str) -> sqlx::Result<Option<i64>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as("SELECT r.qty, i.unit_cost_x1000 FROM recipe_items r JOIN ingredients i ON i.id = r.ingredient_id WHERE r.menu_id = ?")
        .bind(menu_id)
        .fetch_all(conn)
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(rows.iter().map(|(q, c)| q * c).sum::<i64>() / 1000))
}

/// Consume ingredients for a fully paid order. Called inside the payment transaction.
pub async fn deduct_for_order(conn: &mut SqliteConnection, order_id: &str, user: &str) -> sqlx::Result<()> {
    let uses: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT r.ingredient_id, SUM(r.qty * i.qty), ig.unit_cost_x1000 FROM order_items i JOIN recipe_items r ON r.menu_id = i.menu_id JOIN ingredients ig ON ig.id = r.ingredient_id \
         WHERE i.order_id = ? AND i.voided = 0 GROUP BY r.ingredient_id",
    )
    .bind(order_id)
    .fetch_all(&mut *conn)
    .await?;
    let now = clock::now();
    for (ing, qty, cost) in uses {
        sqlx::query("UPDATE ingredients SET stock = stock - ? WHERE id = ?")
            .bind(qty)
            .bind(&ing)
            .execute(&mut *conn)
            .await?;
        sqlx::query("INSERT INTO stock_movements (id, ingredient_id, kind, qty, unit_cost_x1000, ref, user_id, created_at) VALUES (?,?,?,?,?,?,?,?)")
            .bind(rand_id::id())
            .bind(&ing)
            .bind("sale")
            .bind(-qty)
            .bind(cost)
            .bind(order_id)
            .bind(user)
            .bind(now)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn page(st: &AppState, staff: &Staff, error: Option<String>, notice: Option<String>) -> AppResult<Response> {
    let rows: Vec<(String, String, String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, name, unit, stock, unit_cost_x1000, low_at, active FROM ingredients ORDER BY active DESC, name",
    )
    .fetch_all(&st.db)
    .await?;
    let value: i64 = rows.iter().map(|r| r.3.max(0) * r.4).sum::<i64>() / 1000;
    let ingredients = rows
        .into_iter()
        .map(|(id, name, unit, stock, cost, low_at, active)| IngredientView {
            id,
            name,
            unit,
            stock,
            cost: cost_fmt(cost),
            low: stock <= low_at && active == 1,
            active: active == 1,
        })
        .collect();
    Ok(render(IngredientsPage {
        ctx: Ctx::staff(st, staff, "ingredients").await?,
        error,
        notice,
        ingredients,
        value: rupiah(value),
    })?
    .into_response())
}

pub async fn get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    page(&st, &staff, None, None).await
}

#[derive(Deserialize)]
pub struct NewIngredient {
    name: String,
    unit: String,
    low_at: Option<i64>,
}

pub async fn add(State(st): State<AppState>, staff: Staff, Form(f): Form<NewIngredient>) -> AppResult<Response> {
    owner(&staff)?;
    let name = clean(&f.name, 40);
    if name.is_empty() || !["gram", "ml", "pcs"].contains(&f.unit.as_str()) {
        return page(&st, &staff, Some("Nama dan satuan wajib diisi.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let id = rand_id::id();
    if sqlx::query("INSERT INTO ingredients (id, name, unit, low_at) VALUES (?,?,?,?)")
        .bind(&id)
        .bind(&name)
        .bind(&f.unit)
        .bind(f.low_at.unwrap_or(0).max(0))
        .execute(&mut *tx)
        .await
        .is_err()
    {
        drop(tx);
        return page(&st, &staff, Some("Nama bahan sudah dipakai.".into()), None).await;
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "ingredient.created", "ingredient", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/ingredients").into_response())
}

#[derive(Deserialize)]
pub struct Buy {
    qty: i64,
    total_price: i64,
}

/// Purchase: stock goes up and the moving-average unit cost is recomputed from the invoice total.
pub async fn buy(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<Buy>,
) -> AppResult<Response> {
    owner(&staff)?;
    if f.qty <= 0 || f.total_price < 0 || f.qty > 100_000_000 || f.total_price > 10_000_000_000 {
        return page(
            &st,
            &staff,
            Some("Jumlah dan harga pembelian tidak valid.".into()),
            None,
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let (stock, cost): (i64, i64) = sqlx::query_as("SELECT stock, unit_cost_x1000 FROM ingredients WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let buy_cost_x1000 = f.total_price * 1000 / f.qty;
    let on_hand = stock.max(0);
    let new_cost = if on_hand + f.qty > 0 {
        (on_hand * cost + f.qty * buy_cost_x1000) / (on_hand + f.qty)
    } else {
        buy_cost_x1000
    };
    sqlx::query("UPDATE ingredients SET stock = stock + ?, unit_cost_x1000 = ? WHERE id = ?")
        .bind(f.qty)
        .bind(new_cost)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO stock_movements (id, ingredient_id, kind, qty, unit_cost_x1000, user_id, created_at) VALUES (?,?,?,?,?,?,?)")
        .bind(rand_id::id())
        .bind(&id)
        .bind("purchase")
        .bind(f.qty)
        .bind(buy_cost_x1000)
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "stock.purchase", "ingredient", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/ingredients").into_response())
}

#[derive(Deserialize)]
pub struct Count {
    counted: i64,
    note: Option<String>,
}

/// Stock opname: set the physical count; the difference is recorded as a movement.
pub async fn opname(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<Count>,
) -> AppResult<Response> {
    owner(&staff)?;
    if !(0..=1_000_000_000).contains(&f.counted) {
        return page(&st, &staff, Some("Jumlah hitung tidak valid.".into()), None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let (stock, cost): (i64, i64) = sqlx::query_as("SELECT stock, unit_cost_x1000 FROM ingredients WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let diff = f.counted - stock;
    sqlx::query("UPDATE ingredients SET stock = ? WHERE id = ?")
        .bind(f.counted)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO stock_movements (id, ingredient_id, kind, qty, unit_cost_x1000, note, user_id, created_at) VALUES (?,?,?,?,?,?,?,?)")
        .bind(rand_id::id())
        .bind(&id)
        .bind("opname")
        .bind(diff)
        .bind(cost)
        .bind(f.note.as_deref().map(|n| clean(n, 120)).filter(|n| !n.is_empty()))
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    let detail = format!("system {stock} counted {} diff {diff}", f.counted);
    let mut e = Entry::new(Some(&staff.id), "stock.opname", "ingredient", Some(&id));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/ingredients").into_response())
}

#[derive(Deserialize)]
pub struct Waste {
    qty: i64,
    reason: String,
}

pub async fn waste(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<Waste>,
) -> AppResult<Response> {
    owner(&staff)?;
    let reason = clean(&f.reason, 120);
    if f.qty <= 0 || reason.is_empty() {
        return page(
            &st,
            &staff,
            Some("Jumlah dan alasan pemborosan wajib diisi.".into()),
            None,
        )
        .await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let cost: i64 = sqlx::query_scalar("SELECT unit_cost_x1000 FROM ingredients WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE ingredients SET stock = stock - ? WHERE id = ?")
        .bind(f.qty)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO stock_movements (id, ingredient_id, kind, qty, unit_cost_x1000, note, user_id, created_at) VALUES (?,?,?,?,?,?,?,?)")
        .bind(rand_id::id())
        .bind(&id)
        .bind("waste")
        .bind(-f.qty)
        .bind(cost)
        .bind(&reason)
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    let mut e = Entry::new(Some(&staff.id), "stock.waste", "ingredient", Some(&id));
    e.reason = Some(&reason);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/ingredients").into_response())
}

// ---- Recipe per menu item ------------------------------------------------------------------------

async fn recipe_page(st: &AppState, staff: &Staff, menu_id: &str, error: Option<String>) -> AppResult<Response> {
    let name: String = sqlx::query_scalar("SELECT name FROM menu_items WHERE id = ?")
        .bind(menu_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let lines: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT i.id, i.name, i.unit, r.qty, i.unit_cost_x1000 FROM recipe_items r JOIN ingredients i ON i.id = r.ingredient_id WHERE r.menu_id = ? ORDER BY i.name",
    )
    .bind(menu_id)
    .fetch_all(&st.db)
    .await?;
    let total: i64 = lines.iter().map(|l| l.3 * l.4).sum::<i64>() / 1000;
    let options: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, unit FROM ingredients WHERE active = 1 ORDER BY name")
            .fetch_all(&st.db)
            .await?;
    Ok(render(RecipePage {
        ctx: Ctx::staff(st, staff, "menu").await?,
        error,
        menu_id: menu_id.into(),
        menu_name: name,
        lines: lines
            .into_iter()
            .map(|(id, n, u, q, c)| (id, n, u, q, rupiah(q * c / 1000)))
            .collect(),
        total: rupiah(total),
        options,
    })?
    .into_response())
}

pub async fn recipe_get(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    owner(&staff)?;
    recipe_page(&st, &staff, &id, None).await
}

#[derive(Deserialize)]
pub struct RecipeLine {
    ingredient_id: String,
    qty: i64,
}

pub async fn recipe_set(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Form(f): Form<RecipeLine>,
) -> AppResult<Response> {
    owner(&staff)?;
    if f.qty <= 0 || f.qty > 1_000_000 {
        return recipe_page(&st, &staff, &id, Some("Jumlah per porsi tidak valid.".into())).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ingredients WHERE id = ? AND active = 1) AND EXISTS(SELECT 1 FROM menu_items WHERE id = ?)").bind(&f.ingredient_id).bind(&id).fetch_one(&mut *tx).await?;
    if !ok {
        return Err(AppError::NotFound);
    }
    sqlx::query("INSERT INTO recipe_items (menu_id, ingredient_id, qty) VALUES (?,?,?) ON CONFLICT(menu_id, ingredient_id) DO UPDATE SET qty = excluded.qty").bind(&id).bind(&f.ingredient_id).bind(f.qty).execute(&mut *tx).await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "recipe.updated", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{id}/recipe")).into_response())
}

pub async fn recipe_remove(
    State(st): State<AppState>,
    staff: Staff,
    Path((id, ing)): Path<(String, String)>,
) -> AppResult<Redirect> {
    owner(&staff)?;
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("DELETE FROM recipe_items WHERE menu_id = ? AND ingredient_id = ?")
        .bind(&id)
        .bind(&ing)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "recipe.updated", "menu_item", Some(&id)),
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/items/{id}/recipe")))
}
