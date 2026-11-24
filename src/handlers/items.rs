// SPDX-License-Identifier: AGPL-3.0-or-later
//! One form to create or edit a menu item: name, description, price, cost, stock and photo together.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    error::{AppError, AppResult},
    handlers::{clean, more},
    rand_id,
    views::*,
};
use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

#[derive(Default)]
struct Parsed {
    category_id: String,
    name: String,
    description: String,
    price: String,
    cost: String,
    stock: String,
    remove_photo: bool,
    photo: Option<Vec<u8>>,
}

async fn parse(mut mp: Multipart) -> AppResult<Parsed> {
    let bad = |_| AppError::Invalid("Unggahan tidak valid atau foto lebih dari 3 MB.".into());
    let mut p = Parsed::default();
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "photo" => {
                let b = field.bytes().await.map_err(bad)?;
                if b.len() > 3 * 1024 * 1024 {
                    return Err(AppError::Invalid("Foto terlalu besar (maksimal 3 MB).".into()));
                }
                if !b.is_empty() {
                    p.photo = Some(b.to_vec());
                }
            }
            _ => {
                let t = field.text().await.map_err(bad)?;
                match name.as_str() {
                    "category_id" => p.category_id = t,
                    "name" => p.name = t,
                    "description" => p.description = t,
                    "price" => p.price = t,
                    "cost" => p.cost = t,
                    "stock" => p.stock = t,
                    "remove_photo" => p.remove_photo = t == "1",
                    _ => {}
                }
            }
        }
    }
    Ok(p)
}

fn int(s: &str, max: i64) -> Option<Option<i64>> {
    let s = s.trim();
    if s.is_empty() {
        return Some(None);
    }
    s.parse::<i64>().ok().filter(|v| (0..=max).contains(v)).map(Some)
}

async fn categories(st: &AppState) -> AppResult<Vec<CategoryRow>> {
    Ok(
        sqlx::query_as("SELECT id, name FROM categories WHERE active = 1 ORDER BY sort_order, name")
            .fetch_all(&st.db)
            .await?,
    )
}

async fn page(
    st: &AppState,
    staff: &Staff,
    id: Option<String>,
    f: ItemFormView,
    error: Option<String>,
) -> AppResult<Response> {
    Ok(render(OwnerItemForm {
        ctx: Ctx::staff(st, staff, "menu").await?,
        error,
        categories: categories(st).await?,
        id,
        f,
    })?
    .into_response())
}

pub async fn new_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    page(&st, &staff, None, ItemFormView::default(), None).await
}

pub async fn edit_get(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    owner(&staff)?;
    let r: ItemFormRow = sqlx::query_as(
        "SELECT category_id, name, COALESCE(description,'') AS description, price, cost, stock, photo FROM menu_items WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&st.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let f = ItemFormView {
        category_id: r.category_id,
        name: r.name,
        description: r.description,
        price: r.price.to_string(),
        cost: r.cost.to_string(),
        stock: r.stock.map(|s| s.to_string()).unwrap_or_default(),
        photo: r.photo,
    };
    page(&st, &staff, Some(id), f, None).await
}

pub async fn create(State(st): State<AppState>, staff: Staff, mp: Multipart) -> AppResult<Response> {
    save(st, staff, None, mp).await
}

pub async fn update(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    mp: Multipart,
) -> AppResult<Response> {
    save(st, staff, Some(id), mp).await
}

async fn save(st: AppState, staff: Staff, id: Option<String>, mp: Multipart) -> AppResult<Response> {
    owner(&staff)?;
    let p = parse(mp).await?;
    let echo = ItemFormView {
        category_id: p.category_id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        price: p.price.clone(),
        cost: p.cost.clone(),
        stock: p.stock.clone(),
        photo: None,
    };
    let name = clean(&p.name, 80);
    let description = clean(&p.description, 240);
    let (Some(Some(price)), Some(cost), Some(stock)) = (
        int(&p.price, 100_000_000),
        int(&p.cost, 100_000_000),
        int(&p.stock, 1_000_000),
    ) else {
        return page(
            &st,
            &staff,
            id,
            echo,
            Some("Harga, HPP, atau stok bukan angka yang valid.".into()),
        )
        .await;
    };
    if name.is_empty() {
        return page(&st, &staff, id, echo, Some("Nama menu wajib diisi.".into())).await;
    }
    let cat_ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM categories WHERE id = ? AND active = 1)")
        .bind(&p.category_id)
        .fetch_one(&st.db)
        .await?;
    if !cat_ok {
        return page(
            &st,
            &staff,
            id,
            echo,
            Some("Pilih kategori. Buat kategori dulu bila belum ada.".into()),
        )
        .await;
    }
    // Re-encode first (CPU work, outside the write lock); only a valid image ever touches disk.
    let mut new_photo: Option<String> = None;
    if let Some(bytes) = p.photo {
        let dir = st
            .cfg
            .uploads_dir()
            .ok_or_else(|| AppError::Invalid("Penyimpanan foto tidak tersedia pada database ini.".into()))?;
        match tokio::task::spawn_blocking(move || more::reencode(&bytes)).await? {
            Ok(jpeg) => {
                let n = format!("{}.jpg", rand_id::id());
                tokio::fs::create_dir_all(&dir).await?;
                tokio::fs::write(dir.join(&n), jpeg).await?;
                new_photo = Some(n);
            }
            Err(m) => return page(&st, &staff, id, echo, Some(m.into())).await,
        }
    }
    let cost = cost.unwrap_or(0);
    let mut tx = crate::db::begin_write(&st.db).await?;
    let (target, action, old_photo) = match id {
        Some(id) => {
            let old: Option<Option<String>> = sqlx::query_scalar("SELECT photo FROM menu_items WHERE id = ?")
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?;
            let Some(old) = old else {
                drop_file(&st, new_photo.as_deref()).await;
                return Err(AppError::NotFound);
            };
            let photo = if new_photo.is_some() {
                new_photo.clone()
            } else if p.remove_photo {
                None
            } else {
                old.clone()
            };
            sqlx::query(
                "UPDATE menu_items SET category_id=?, name=?, description=?, price=?, cost=?, stock=?, photo=?, \
                 available = CASE WHEN ? IS NOT NULL AND ? = 0 THEN 0 WHEN ? > 0 THEN 1 ELSE available END WHERE id=?",
            )
            .bind(&p.category_id)
            .bind(&name)
            .bind((!description.is_empty()).then_some(&description))
            .bind(price)
            .bind(cost)
            .bind(stock)
            .bind(photo)
            .bind(stock)
            .bind(stock)
            .bind(stock.unwrap_or(0))
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            (id, "menu.edited", old)
        }
        None => {
            let id = rand_id::id();
            sqlx::query(
                "INSERT INTO menu_items (id, category_id, name, description, price, cost, stock, photo) VALUES (?,?,?,?,?,?,?,?)",
            )
            .bind(&id)
            .bind(&p.category_id)
            .bind(&name)
            .bind((!description.is_empty()).then_some(&description))
            .bind(price)
            .bind(cost)
            .bind(stock)
            .bind(&new_photo)
            .execute(&mut *tx)
            .await?;
            (id, "menu.created", None)
        }
    };
    audit::record(&mut tx, Entry::new(Some(&staff.id), action, "menu_item", Some(&target))).await?;
    tx.commit().await?;
    if (new_photo.is_some() || p.remove_photo)
        && let Some(old) = old_photo
    {
        drop_file(&st, Some(&old)).await;
    }
    Ok(Redirect::to("/owner/menu").into_response())
}

async fn drop_file(st: &AppState, name: Option<&str>) {
    if let (Some(n), Some(dir)) = (name, st.cfg.uploads_dir())
        && more::valid_upload_name(n)
    {
        let _ = tokio::fs::remove_file(dir.join(n)).await;
    }
}
