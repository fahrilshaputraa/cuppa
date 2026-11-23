// SPDX-License-Identifier: AGPL-3.0-or-later
//! Staff authentication: Argon2id-hashed secrets, lockout, server-side sessions in an HttpOnly cookie.

use crate::{AppState, clock, error::AppError};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use axum::{extract::FromRequestParts, http::request::Parts};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

pub const COOKIE: &str = "cuppa_session";
const SESSION_TTL: i64 = 12 * 3600;
const MAX_FAILS: i64 = 5;
const LOCK_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Cashier,
    Kitchen,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "cashier" => Some(Self::Cashier),
            "kitchen" => Some(Self::Kitchen),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Cashier => "cashier",
            Self::Kitchen => "kitchen",
        }
    }
    pub fn home(self) -> &'static str {
        match self {
            Self::Owner => "/owner",
            Self::Cashier => "/cashier",
            Self::Kitchen => "/kitchen",
        }
    }
}

/// Fine-grained permissions. Role defaults can be granted/revoked per user (FR-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perm {
    Reports,
    Discount,
    Refund,
    CashMove,
    History,
    SoldOut,
}

impl Perm {
    pub const ALL: [Perm; 6] = [
        Perm::Reports,
        Perm::Discount,
        Perm::Refund,
        Perm::CashMove,
        Perm::History,
        Perm::SoldOut,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Perm::Reports => "reports",
            Perm::Discount => "discount",
            Perm::Refund => "refund",
            Perm::CashMove => "cash_move",
            Perm::History => "history",
            Perm::SoldOut => "sold_out",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Perm::Reports => "Lihat laporan",
            Perm::Discount => "Beri diskon",
            Perm::Refund => "Refund",
            Perm::CashMove => "Kas masuk/keluar",
            Perm::History => "Riwayat pesanan",
            Perm::SoldOut => "Tandai menu habis",
        }
    }

    pub fn from_key(k: &str) -> Option<Perm> {
        Perm::ALL.into_iter().find(|p| p.key() == k)
    }

    pub fn default_for(self, role: Role) -> bool {
        match role {
            Role::Owner => true,
            Role::Cashier => matches!(self, Perm::Discount | Perm::CashMove | Perm::History | Perm::SoldOut),
            Role::Kitchen => matches!(self, Perm::SoldOut),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Staff {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub perms: Vec<Perm>,
}

impl Staff {
    /// Authorization is checked on the server for every handler, never just hidden in the UI.
    pub fn require(&self, allowed: &[Role]) -> Result<(), AppError> {
        if allowed.contains(&self.role) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    pub fn can(&self, p: Perm) -> bool {
        self.perms.contains(&p)
    }

    pub fn need(&self, p: Perm) -> Result<(), AppError> {
        if self.can(p) { Ok(()) } else { Err(AppError::Forbidden) }
    }
}

/// Owner: passphrase of 8+ characters. Cashier/kitchen: numeric PIN of 4–8 digits.
pub fn validate_secret(role: Role, s: &str) -> Result<(), String> {
    match role {
        Role::Owner if s.chars().count() >= 8 && s.chars().count() <= 128 => Ok(()),
        Role::Owner => Err("Kata sandi owner minimal 8 karakter.".into()),
        _ if (4..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
        _ => Err("PIN harus 4–8 digit angka.".into()),
    }
}

pub async fn hash_secret(secret: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(secret.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("hash: {e}"))
    })
    .await?
}

async fn verify_secret(secret: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .verify_password(secret.as_bytes(), hash.as_str())
            .is_ok()
    })
    .await
    .unwrap_or(false)
}

/// Unknown usernames cost the same Argon2 work as known ones (no user enumeration by timing).
async fn dummy_verify(secret: String) {
    static DUMMY: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let hash = DUMMY
        .get_or_init(|| async { hash_secret("dummy-secret".into()).await.unwrap_or_default() })
        .await
        .clone();
    let _ = verify_secret(secret, hash).await;
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    name: String,
    role: String,
    pin_hash: String,
    failed_attempts: i64,
    locked_until: i64,
}

pub enum CredError {
    Invalid,
    Locked,
}

/// Verify name + secret with lockout. Returns the authenticated staff member.
pub async fn check_credentials(pool: &SqlitePool, name: &str, secret: &str) -> Result<Staff, CredError> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, name, role, pin_hash, failed_attempts, locked_until FROM users WHERE name = ? COLLATE NOCASE AND active = 1",
    )
    .bind(name.trim())
    .fetch_optional(pool)
    .await
    .map_err(|_| CredError::Invalid)?;
    let Some(u) = row else {
        dummy_verify(secret.to_string()).await;
        return Err(CredError::Invalid);
    };
    let now = clock::now();
    if u.locked_until > now {
        return Err(CredError::Locked);
    }
    if verify_secret(secret.to_string(), u.pin_hash.clone()).await {
        let _ = sqlx::query("UPDATE users SET failed_attempts = 0, locked_until = 0 WHERE id = ?")
            .bind(&u.id)
            .execute(pool)
            .await;
        let role = Role::parse(&u.role).ok_or(CredError::Invalid)?;
        return Ok(Staff {
            id: u.id,
            name: u.name,
            role,
            perms: Vec::new(),
        });
    }
    let fails = u.failed_attempts + 1;
    let (fails, lock) = if fails >= MAX_FAILS {
        (0, now + LOCK_SECS)
    } else {
        (fails, 0)
    };
    let _ = sqlx::query("UPDATE users SET failed_attempts = ?, locked_until = ? WHERE id = ?")
        .bind(fails)
        .bind(lock)
        .bind(&u.id)
        .execute(pool)
        .await;
    if lock > 0 {
        let mut c = match pool.acquire().await {
            Ok(c) => c,
            Err(_) => return Err(CredError::Locked),
        };
        let _ = crate::audit::record(
            &mut c,
            crate::audit::Entry::new(Some(&u.id), "auth.locked", "user", Some(&u.id)),
        )
        .await;
        return Err(CredError::Locked);
    }
    Err(CredError::Invalid)
}

pub async fn create_session(pool: &SqlitePool, user_id: &str) -> anyhow::Result<String> {
    let token = crate::rand_id::hex_token(32);
    let now = clock::now();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen) VALUES (?,?,?,?,?)")
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(now)
        .bind(now + SESSION_TTL)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(token)
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie(secure: bool) -> String {
    format!(
        "{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

impl FromRequestParts<AppState> for Staff {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie_value(&parts.headers, COOKIE).ok_or(AppError::Login)?;
        let now = clock::now();
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.name, u.role FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ? AND s.expires_at > ? AND u.active = 1",
        )
        .bind(hash_token(token))
        .bind(now)
        .fetch_optional(&st.db)
        .await?;
        let (id, name, role) = row.ok_or(AppError::Login)?;
        let role = Role::parse(&role).ok_or(AppError::Login)?;
        let overrides: Vec<(String, i64)> =
            sqlx::query_as("SELECT perm, allowed FROM user_permission_overrides WHERE user_id = ?")
                .bind(&id)
                .fetch_all(&st.db)
                .await?;
        let perms = Perm::ALL
            .into_iter()
            .filter(|p| match overrides.iter().find(|(k, _)| k == p.key()) {
                Some((_, allowed)) => *allowed == 1,
                None => p.default_for(role),
            })
            .collect();
        Ok(Staff { id, name, role, perms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_rules() {
        assert!(validate_secret(Role::Cashier, "1234").is_ok());
        assert!(validate_secret(Role::Cashier, "12a4").is_err());
        assert!(validate_secret(Role::Cashier, "123").is_err());
        assert!(validate_secret(Role::Owner, "short").is_err());
        assert!(validate_secret(Role::Owner, "long enough pass").is_ok());
    }

    #[test]
    fn cookie_is_hardened() {
        let c = session_cookie("abc", true);
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict") && c.contains("Secure"));
    }
}
