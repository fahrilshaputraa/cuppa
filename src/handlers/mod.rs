// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod cashier;
pub mod customer;
pub mod orders;
pub mod owner;
pub mod receipt;
pub mod reports;
pub mod session;

use crate::{error::AppError, money::TaxRules, views::SettingsRow};
use sqlx::SqliteConnection;

pub async fn load_settings(conn: &mut SqliteConnection) -> Result<SettingsRow, AppError> {
    Ok(sqlx::query_as(
        "SELECT outlet_name, tax_bp, service_bp, tax_inclusive, service_in_tax_base, qr_ordering, confirm_timeout_min, printer_host, paper_cols, receipt_footer, open_drawer_on_cash FROM outlet_settings WHERE id = 1",
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

/// Collect `qty_<menu id>` fields into (menu_id, qty), ignoring zeros. Bounded to stop abuse.
pub fn parse_qty(fields: Vec<(String, String)>, max_qty: i64) -> Result<Vec<(String, i64)>, AppError> {
    let mut out = Vec::new();
    for (k, v) in fields {
        if let Some(id) = k.strip_prefix("qty_") {
            let q: i64 = v
                .trim()
                .parse()
                .map_err(|_| AppError::Invalid("Jumlah tidak valid.".into()))?;
            if q < 0 || q > max_qty {
                return Err(AppError::Invalid("Jumlah di luar batas.".into()));
            }
            if q > 0 {
                if id.len() > 64 || out.len() >= 40 {
                    return Err(AppError::Invalid("Terlalu banyak item.".into()));
                }
                out.push((id.to_string(), q));
            }
        }
    }
    if out.is_empty() {
        return Err(AppError::Invalid("Pilih minimal satu menu.".into()));
    }
    Ok(out)
}

pub fn clean(s: &str, max: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}
