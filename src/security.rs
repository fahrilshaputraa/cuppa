// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cross-cutting protections: security headers, CSRF origin check, rate limiting.

use crate::{AppState, error::AppError};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderName, HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// Strict headers on every response. No inline script/style is ever needed by Cuppa's own UI.
pub async fn headers(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let is_static = req.uri().path().starts_with("/static/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, k: &'static str, v: &'static str| {
        h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
    };
    set(h, "content-security-policy", CSP);
    set(h, "x-content-type-options", "nosniff");
    set(h, "x-frame-options", "DENY");
    set(h, "referrer-policy", "no-referrer");
    set(
        h,
        "permissions-policy",
        "camera=(), microphone=(), geolocation=(), payment=()",
    );
    set(h, "cross-origin-opener-policy", "same-origin");
    set(h, "cross-origin-resource-policy", "same-origin");
    if st.cfg.cookie_secure {
        set(h, "strict-transport-security", "max-age=31536000; includeSubDomains");
    }
    if !is_static && !h.contains_key(header::CACHE_CONTROL) {
        // Pages carry orders and money: never cache them in shared or browser caches.
        set(h, "cache-control", "no-store");
    }
    res
}

/// CSRF defence in depth (the session cookie is also SameSite=Strict):
/// state-changing requests must come from our own origin.
pub async fn csrf(req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let h = req.headers();
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let origin_host = h
        .get(header::ORIGIN)
        .or_else(|| h.get(header::REFERER))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("://").nth(1))
        .map(|rest| rest.split('/').next().unwrap_or(""));
    let ok = match origin_host {
        Some(oh) => !host.is_empty() && oh.eq_ignore_ascii_case(host),
        None => h.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("same-origin"),
    };
    if ok {
        next.run(req).await
    } else {
        AppError::Forbidden.into_response()
    }
}

/// Fixed-window in-memory limiter keyed by an arbitrary string.
#[derive(Default)]
pub struct RateLimiter(Mutex<HashMap<String, Vec<Instant>>>);

impl RateLimiter {
    /// `true` if the call is allowed.
    pub fn allow(&self, key: &str, max: usize, window: Duration) -> bool {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if map.len() > 10_000 {
            map.retain(|_, v| v.last().is_some_and(|t| now.duration_since(*t) < window));
        }
        let hits = map.entry(key.to_string()).or_default();
        hits.retain(|t| now.duration_since(*t) < window);
        if hits.len() >= max {
            return false;
        }
        hits.push(now);
        true
    }
}

pub fn client_ip(st: &AppState, req_headers: &axum::http::HeaderMap, peer: SocketAddr) -> IpAddr {
    if st.cfg.trust_proxy {
        // Our proxy appends the real client as the LAST entry; earlier entries are client-controlled.
        if let Some(ip) = req_headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
        {
            return ip;
        }
    }
    peer.ip()
}

/// Per-IP throttle for public/unauthenticated routes (login, setup, customer ordering).
pub async fn throttle_public(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&st, req.headers(), peer);
    let key = format!("pub:{ip}");
    if !st.limiter.allow(&key, 120, Duration::from_secs(60)) {
        return AppError::TooMany.into_response();
    }
    next.run(req).await
}
