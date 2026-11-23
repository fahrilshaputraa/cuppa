// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod admin;
pub mod cashier;
pub mod customer;
pub mod extras;
pub mod inventory;
pub mod more;
pub mod ops;
pub mod orders;
pub mod owner;
pub mod public;
pub mod receipt;
pub mod reports;
pub mod session;

use crate::{
    AppState,
    error::AppError,
    money::{TaxRules, rupiah},
    views::{GroupView, MenuCategoryView, MenuItemView, MenuRow, ModView, SettingsRow},
};
use sqlx::SqliteConnection;
use std::collections::HashMap;

pub async fn load_settings(conn: &mut SqliteConnection) -> Result<SettingsRow, AppError> {
    Ok(sqlx::query_as(
        "SELECT outlet_name, tax_bp, service_bp, tax_inclusive, service_in_tax_base, qr_ordering, confirm_timeout_min, \
         printer_host, paper_cols, receipt_footer, open_drawer_on_cash, kitchen_mode, wizard_done, open_from, open_to, \
         round_to, cashier_discount_max_bp, theme, require_device, kitchen_printer_host, busy_threshold, busy_extra_min FROM outlet_settings WHERE id = 1",
    )
    .fetch_one(conn)
    .await?)
}

pub fn rules(s: &SettingsRow) -> TaxRules {
    TaxRules {
        tax_bp: s.tax_bp,
        service_bp: s.service_bp,
        tax_inclusive: s.tax_inclusive == 1,
        service_in_tax_base: s.service_in_tax_base == 1,
    }
}

pub fn clean(s: &str, max: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Is the café open at `now`? Supports overnight ranges (e.g. 18:00–02:00). No hours set = always open.
pub fn is_open(s: &SettingsRow, hm_now: &str) -> bool {
    let (Some(from), Some(to)) = (s.open_from.as_deref(), s.open_to.as_deref()) else {
        return true;
    };
    if from <= to {
        hm_now >= from && hm_now < to
    } else {
        hm_now >= from || hm_now < to
    }
}

pub fn hours_text(s: &SettingsRow) -> String {
    match (&s.open_from, &s.open_to) {
        (Some(a), Some(b)) => format!("{a}–{b}"),
        _ => String::new(),
    }
}

/// Active menu grouped by category, with modifier groups attached. Used by customer and cashier screens.
pub async fn load_menu(st: &AppState) -> Result<Vec<MenuCategoryView>, AppError> {
    let rows: Vec<MenuRow> = sqlx::query_as(
        "SELECT m.id, m.name, m.price, m.available, m.stock, m.photo, c.name AS category FROM menu_items m JOIN categories c ON c.id = m.category_id \
         WHERE m.active = 1 AND c.active = 1 ORDER BY c.sort_order, c.name, m.name",
    )
    .fetch_all(&st.db)
    .await?;
    let groups: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, menu_id, name, required, max_select FROM modifier_groups ORDER BY required DESC, name",
    )
    .fetch_all(&st.db)
    .await?;
    let mods: Vec<(String, String, String, i64)> =
        sqlx::query_as("SELECT id, group_id, name, price FROM modifiers WHERE active = 1 ORDER BY price, name")
            .fetch_all(&st.db)
            .await?;
    let mut by_menu: HashMap<String, Vec<GroupView>> = HashMap::new();
    for (gid, menu_id, name, required, max_select) in groups {
        let mv: Vec<ModView> = mods
            .iter()
            .filter(|m| m.1 == gid)
            .map(|m| ModView {
                id: m.0.clone(),
                name: m.2.clone(),
                price_fmt: if m.3 > 0 {
                    format!("+{}", rupiah(m.3))
                } else {
                    String::new()
                },
                price: m.3,
                active: true,
            })
            .collect();
        if !mv.is_empty() {
            by_menu.entry(menu_id).or_default().push(GroupView {
                id: gid,
                name,
                required: required == 1,
                max_select,
                mods: mv,
            });
        }
    }
    let mut out: Vec<MenuCategoryView> = Vec::new();
    for r in rows {
        let item = MenuItemView {
            photo: r.photo,
            groups: by_menu.remove(&r.id).unwrap_or_default(),
            stock: r.stock,
            id: r.id,
            name: r.name,
            price: r.price,
            price_fmt: rupiah(r.price),
            available: r.available,
        };
        match out.last_mut() {
            Some(c) if c.name == r.category => c.items.push(item),
            _ => out.push(MenuCategoryView {
                name: r.category,
                items: vec![item],
            }),
        }
    }
    Ok(out)
}
