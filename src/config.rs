// SPDX-License-Identifier: AGPL-3.0-or-later
//! Runtime configuration, read once from the environment. Defaults are the secure choice.

use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub public_url: String,
    /// `Secure` flag on the session cookie. Keep `true` whenever served over HTTPS.
    pub cookie_secure: bool,
    /// Trust the last `X-Forwarded-For` entry. Only enable behind your own reverse proxy.
    pub trust_proxy: bool,
    /// Outlet UTC offset in hours (Asia/Jakarta = 7).
    pub tz_offset_hours: i8,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let get = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let flag = |k: &str, d: bool| match std::env::var(k).ok().as_deref() {
            Some("1" | "true" | "yes") => true,
            Some("0" | "false" | "no") => false,
            _ => d,
        };
        let public_url = get("CUPPA_PUBLIC_URL", "http://localhost:8080");
        Ok(Self {
            // Loopback by default: exposing the app is an explicit decision (reverse proxy / tunnel).
            bind: get("CUPPA_BIND", "127.0.0.1:8080").parse()?,
            database_url: get("DATABASE_URL", "sqlite://data/cuppa.db"),
            cookie_secure: flag("CUPPA_COOKIE_SECURE", public_url.starts_with("https://")),
            public_url,
            trust_proxy: flag("CUPPA_TRUST_PROXY", false),
            tz_offset_hours: get("CUPPA_TZ_OFFSET_HOURS", "7").parse()?,
        })
    }

    /// Where re-encoded menu photos live (next to the database). `None` for in-memory databases.
    pub fn uploads_dir(&self) -> Option<std::path::PathBuf> {
        crate::backup::dir_for(&self.database_url).map(|b| b.with_file_name("uploads"))
    }

    pub fn for_tests() -> Self {
        Self {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: "sqlite::memory:".into(),
            public_url: "http://localhost:8080".into(),
            cookie_secure: false,
            trust_proxy: false,
            tz_offset_hours: 7,
        }
    }
}
