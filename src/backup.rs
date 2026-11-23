// SPDX-License-Identifier: AGPL-3.0-or-later
//! Consistent SQLite backups (`VACUUM INTO`, safe while the app is running) with daily rotation.

use crate::{AppState, clock};
use std::path::{Path, PathBuf};

const KEEP: usize = 14;

pub fn dir_for(database_url: &str) -> Option<PathBuf> {
    let p = database_url.strip_prefix("sqlite://")?;
    if p.starts_with(':') || p.contains(":memory:") {
        return None;
    }
    Some(
        Path::new(p)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .join("backups"),
    )
}

/// Write a snapshot to `target` (must not exist). The path is bound as a parameter, never formatted into SQL.
pub async fn snapshot(st: &AppState, target: &Path) -> anyhow::Result<()> {
    if let Some(d) = target.parent() {
        tokio::fs::create_dir_all(d).await?;
    }
    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().to_string())
        .execute(&st.db)
        .await?;
    Ok(())
}

/// Daily backup + keep the newest `KEEP`. Called from the housekeeping loop; cheap when already done.
pub async fn daily(st: &AppState) -> anyhow::Result<()> {
    let Some(dir) = dir_for(&st.cfg.database_url) else {
        return Ok(());
    };
    let name = format!("cuppa-{}.db", clock::day(clock::now(), st.cfg.tz_offset_hours));
    let target = dir.join(&name);
    if target.exists() {
        return Ok(());
    }
    snapshot(st, &target).await?;
    tracing::info!(file = %target.display(), "daily backup written");
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    files.sort();
    while files.len() > KEEP {
        let _ = std::fs::remove_file(files.remove(0));
    }
    Ok(())
}

/// Open a backup read-only, run SQLite's integrity check and print what it contains. Exit code != 0 on failure.
pub async fn verify(path: &str) -> anyhow::Result<()> {
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?.read_only(true);
    let pool = sqlx::SqlitePool::connect_with(opts).await?;
    let check: String = sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&pool).await?;
    anyhow::ensure!(check == "ok", "integrity check failed: {check}");
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM users), (SELECT COUNT(*) FROM menu_items), (SELECT COUNT(*) FROM orders), \
         (SELECT COUNT(*) FROM payments), (SELECT COUNT(*) FROM audit_log)",
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "users: {}, menu items: {}, orders: {}, payments: {}, audit rows: {}",
        counts.0, counts.1, counts.2, counts.3, counts.4
    );
    println!("OK: backup is readable and consistent");
    Ok(())
}
