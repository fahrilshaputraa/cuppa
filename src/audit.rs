// SPDX-License-Identifier: AGPL-3.0-or-later
//! Append-only audit trail. The database refuses UPDATE/DELETE on this table (see migration).

use crate::clock;
use sqlx::SqliteConnection;

pub struct Entry<'a> {
    pub actor: Option<&'a str>,
    pub approver: Option<&'a str>,
    pub action: &'a str,
    pub entity: &'a str,
    pub entity_id: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub reason: Option<&'a str>,
}

impl<'a> Entry<'a> {
    pub fn new(actor: Option<&'a str>, action: &'a str, entity: &'a str, entity_id: Option<&'a str>) -> Self {
        Self {
            actor,
            approver: None,
            action,
            entity,
            entity_id,
            detail: None,
            reason: None,
        }
    }
}

/// Write inside the caller's transaction so the audit row commits atomically with the change.
pub async fn record(conn: &mut SqliteConnection, e: Entry<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (at, actor_id, approver_id, action, entity, entity_id, detail, reason) VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(clock::now())
    .bind(e.actor)
    .bind(e.approver)
    .bind(e.action)
    .bind(e.entity)
    .bind(e.entity_id)
    .bind(e.detail)
    .bind(e.reason)
    .execute(conn)
    .await?;
    Ok(())
}
