// SPDX-License-Identifier: AGPL-3.0-or-later
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;

/// Open the pool (WAL, foreign keys ON, busy timeout) and run embedded migrations.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    if let Some(path) = url.strip_prefix("sqlite://")
        && let Some(dir) = std::path::Path::new(path).parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    // In-memory databases are per-connection: keep a single one for tests.
    let max = if url.contains(":memory:") { 1 } else { 8 };
    let pool = SqlitePoolOptions::new().max_connections(max).connect_with(opts).await?;
    let migrator = sqlx::migrate!("./migrations");
    // Safe updates (FR-38): if an existing database is about to get new migrations, snapshot it first.
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap_or(-1);
    if applied > 0
        && applied < migrator.migrations.len() as i64
        && let Some(dir) = crate::backup::dir_for(url)
    {
        std::fs::create_dir_all(&dir)?;
        let target = dir.join(format!("pre-migrate-{}.db", crate::clock::now()));
        sqlx::query("VACUUM INTO ?")
            .bind(target.to_string_lossy().to_string())
            .execute(&pool)
            .await?;
        tracing::warn!(file = %target.display(), "database backed up before applying migrations");
    }
    migrator.run(&pool).await?;
    Ok(pool)
}

/// Start a write transaction with `BEGIN IMMEDIATE`: the write lock is taken up front, so concurrent
/// writers queue on the busy timeout instead of failing mid-transaction (and counters stay gap-free).
pub async fn begin_write(pool: &SqlitePool) -> sqlx::Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE").await
}
