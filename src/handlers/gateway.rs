// SPDX-License-Identifier: AGPL-3.0-or-later
//! Payment gateway (Midtrans QRIS) with signed, idempotent webhooks, plus signed outgoing webhooks.
//!
//! Cuppa never holds money: the QR points at the café's own merchant account. The server key and webhook secret are
//! encrypted at rest (see `secrets`). Everything that touches the network is time-limited and SSRF-guarded.

use crate::{
    AppState,
    audit::{self, Entry},
    auth::{Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::{clean, orders},
    money::rupiah,
    rand_id, secrets,
    views::*,
};
use axum::{
    Form,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::{net::IpAddr, time::Duration};

fn owner(s: &Staff) -> AppResult<()> {
    s.require(&[Role::Owner])
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Midtrans: signature_key = SHA512(order_id + status_code + gross_amount + server_key), lowercase hex.
pub fn midtrans_signature(order_id: &str, status_code: &str, gross_amount: &str, server_key: &str) -> String {
    hex::encode(Sha512::digest(
        format!("{order_id}{status_code}{gross_amount}{server_key}").as_bytes(),
    ))
}

/// `"55000.00"` -> `55000`. Anything with a non-zero fraction is rejected: rupiah has no decimals here.
fn parse_amount(s: &str) -> Option<i64> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if !frac.bytes().all(|b| b == b'0') || int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    int.parse().ok()
}

async fn gateway_key(st: &AppState) -> AppResult<Option<String>> {
    let (provider, enc): (String, Option<String>) =
        sqlx::query_as("SELECT gateway_provider, gateway_key_enc FROM outlet_settings WHERE id = 1")
            .fetch_one(&st.db)
            .await?;
    if provider != "midtrans" {
        return Ok(None);
    }
    Ok(enc.and_then(|e| secrets::decrypt(&st.secret_key, &e)))
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static client config")
}

// ---- Settings page ---------------------------------------------------------------------------

async fn page(
    st: &AppState,
    staff: &Staff,
    error: Option<String>,
    notice: Option<String>,
    new_secret: Option<String>,
) -> AppResult<Response> {
    let (provider, key_set, hook, hook_secret): (String, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as("SELECT gateway_provider, gateway_key_enc, webhook_url, webhook_secret_enc FROM outlet_settings WHERE id = 1").fetch_one(&st.db).await?;
    let tz = st.cfg.tz_offset_hours;
    let charges: Vec<(i64, String, i64, String, i64)> = sqlx::query_as(
        "SELECT o.number, c.provider_order_id, c.amount, c.status, c.created_at FROM gateway_charges c JOIN orders o ON o.id = c.order_id ORDER BY c.created_at DESC LIMIT 15",
    )
    .fetch_all(&st.db)
    .await?;
    let base = st.cfg.public_url.trim_end_matches('/').to_string();
    Ok(render(IntegrationsPage {
        ctx: Ctx::staff(st, staff, "integrations").await?,
        error,
        notice,
        provider,
        key_set: key_set.is_some(),
        webhook_in: format!("{base}/webhooks/midtrans"),
        webhook_url: hook.unwrap_or_default(),
        has_secret: hook_secret.is_some(),
        new_secret,
        charges: charges
            .into_iter()
            .map(|(n, poid, amount, status, at)| {
                (
                    format!("#{n}"),
                    poid,
                    rupiah(amount),
                    status,
                    format!("{} {}", clock::day(at, tz), clock::hm(at, tz)),
                )
            })
            .collect(),
    })?
    .into_response())
}

pub async fn integrations_get(State(st): State<AppState>, staff: Staff) -> AppResult<Response> {
    owner(&staff)?;
    page(&st, &staff, None, None, None).await
}

#[derive(Deserialize)]
pub struct IntegrationsForm {
    provider: String,
    server_key: Option<String>,
    webhook_url: Option<String>,
    rotate_secret: Option<String>,
}

pub async fn integrations_post(
    State(st): State<AppState>,
    staff: Staff,
    Form(f): Form<IntegrationsForm>,
) -> AppResult<Response> {
    owner(&staff)?;
    if !["", "midtrans"].contains(&f.provider.as_str()) {
        return Err(AppError::Invalid("Penyedia tidak dikenal.".into()));
    }
    let url = clean(f.webhook_url.as_deref().unwrap_or(""), 300);
    if !url.is_empty()
        && let Err(e) = validate_webhook_url(&url)
    {
        return page(&st, &staff, Some(e), None, None).await;
    }
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("UPDATE outlet_settings SET gateway_provider = ?, webhook_url = ? WHERE id = 1")
        .bind(&f.provider)
        .bind(if url.is_empty() { None } else { Some(&url) })
        .execute(&mut *tx)
        .await?;
    if let Some(k) = f.server_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        sqlx::query("UPDATE outlet_settings SET gateway_key_enc = ? WHERE id = 1")
            .bind(secrets::encrypt(&st.secret_key, k))
            .execute(&mut *tx)
            .await?;
    }
    let mut shown = None;
    let has: Option<String> = sqlx::query_scalar("SELECT webhook_secret_enc FROM outlet_settings WHERE id = 1")
        .fetch_one(&mut *tx)
        .await?;
    if !url.is_empty() && (has.is_none() || f.rotate_secret.is_some()) {
        let secret = format!("whsec_{}", rand_id::hex_token(24));
        sqlx::query("UPDATE outlet_settings SET webhook_secret_enc = ? WHERE id = 1")
            .bind(secrets::encrypt(&st.secret_key, &secret))
            .execute(&mut *tx)
            .await?;
        shown = Some(secret);
    }
    audit::record(
        &mut tx,
        Entry::new(Some(&staff.id), "integrations.updated", "settings", Some("1")),
    )
    .await?;
    tx.commit().await?;
    page(&st, &staff, None, Some("Integrasi disimpan.".into()), shown).await
}

// ---- Creating a QRIS charge --------------------------------------------------------------------

pub async fn charge_qris(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let Some(key) = gateway_key(&st).await? else {
        return orders::render_detail(
            &st,
            &staff,
            &id,
            Some("Payment gateway belum diatur di Integrasi.".into()),
            None,
        )
        .await;
    };
    let (status, number, total): (String, i64, i64) =
        sqlx::query_as("SELECT status, number, total FROM orders WHERE id = ?")
            .bind(&id)
            .fetch_optional(&st.db)
            .await?
            .ok_or(AppError::NotFound)?;
    if status != "confirmed" && status != "ready" {
        return Err(AppError::Invalid("Pesanan ini tidak bisa dibayar.".into()));
    }
    let mut c = st.db.acquire().await?;
    let remaining = total - orders::paid_so_far(&mut c, &id).await?;
    drop(c);
    if remaining <= 0 {
        return Err(AppError::Invalid("Tidak ada sisa tagihan.".into()));
    }
    let poid = format!("cuppa-{number}-{}", rand_id::hex_token(5));
    let body = json!({"payment_type": "qris", "transaction_details": {"order_id": poid, "gross_amount": remaining}, "qris": {"acquirer": "gopay"}});
    let resp = http()
        .post(format!("{}/v2/charge", st.cfg.midtrans_base.trim_end_matches('/')))
        .basic_auth(&key, Some(""))
        .json(&body)
        .send()
        .await;
    let v: Value = match resp {
        Ok(r) => r.json().await.unwrap_or(Value::Null),
        Err(_) => {
            return orders::render_detail(
                &st,
                &staff,
                &id,
                Some("Gateway tidak dapat dihubungi. Terima pembayaran dengan metode lain atau coba lagi.".into()),
                None,
            )
            .await;
        }
    };
    let qr = v.get("qr_string").and_then(Value::as_str).map(str::to_string);
    let ok_status = v
        .get("status_code")
        .and_then(Value::as_str)
        .is_some_and(|c| c == "201" || c == "200");
    let Some(qr) = qr.filter(|_| ok_status) else {
        tracing::warn!(response = %v, "gateway charge rejected");
        return orders::render_detail(
            &st,
            &staff,
            &id,
            Some("Gateway menolak permintaan. Periksa kunci server di Integrasi.".into()),
            None,
        )
        .await;
    };
    let mut tx = crate::db::begin_write(&st.db).await?;
    sqlx::query("INSERT INTO gateway_charges (id, order_id, provider, provider_order_id, amount, qr_string, created_by, created_at) VALUES (?,?,?,?,?,?,?,?)")
        .bind(rand_id::id())
        .bind(&id)
        .bind("midtrans")
        .bind(&poid)
        .bind(remaining)
        .bind(&qr)
        .bind(&staff.id)
        .bind(clock::now())
        .execute(&mut *tx)
        .await?;
    let detail = format!("{poid} {remaining}");
    let mut e = Entry::new(Some(&staff.id), "gateway.charge_created", "order", Some(&id));
    e.detail = Some(&detail);
    audit::record(&mut tx, e).await?;
    tx.commit().await?;
    orders::render_detail(
        &st,
        &staff,
        &id,
        None,
        Some("QRIS dibuat. Minta pelanggan memindai kode di layar.".into()),
    )
    .await
}

// ---- Settling (webhook + manual status check share one path) -------------------------------------

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Applied,
    AlreadyDone,
    Pending,
    Closed,
    Unapplied,
    Unknown,
    AmountMismatch,
}

/// Apply a provider status to a charge. Idempotent: replays of a settled charge change nothing.
pub async fn settle(
    st: &AppState,
    provider_order_id: &str,
    tx_status: &str,
    fraud: &str,
    gross: Option<i64>,
) -> AppResult<Outcome> {
    let mut tx = crate::db::begin_write(&st.db).await?;
    let ch: Option<(String, String, i64, String, String)> = sqlx::query_as(
        "SELECT id, order_id, amount, status, created_by FROM gateway_charges WHERE provider_order_id = ?",
    )
    .bind(provider_order_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((cid, order_id, amount, status, by)) = ch else {
        return Ok(Outcome::Unknown);
    };
    if status != "pending" {
        return Ok(Outcome::AlreadyDone);
    }
    let paid = tx_status == "settlement" || (tx_status == "capture" && fraud == "accept");
    let closed = matches!(tx_status, "expire" | "deny" | "cancel" | "failure");
    if !paid {
        if closed {
            let new = if tx_status == "expire" { "expired" } else { "failed" };
            sqlx::query("UPDATE gateway_charges SET status = ? WHERE id = ?")
                .bind(new)
                .bind(&cid)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(Outcome::Closed);
        }
        return Ok(Outcome::Pending);
    }
    if gross.is_some_and(|g| g != amount) {
        // Never trust a settlement for a different amount than we asked for.
        let mut e = Entry::new(None, "gateway.amount_mismatch", "order", Some(&order_id));
        let d = format!("{provider_order_id} expected {amount} got {gross:?}");
        e.detail = Some(&d);
        audit::record(&mut tx, e).await?;
        tx.commit().await?;
        return Ok(Outcome::AmountMismatch);
    }
    let (ostatus, total): (String, i64) = sqlx::query_as("SELECT status, total FROM orders WHERE id = ?")
        .bind(&order_id)
        .fetch_one(&mut *tx)
        .await?;
    let already = orders::paid_so_far(&mut tx, &order_id).await?;
    let remaining = total - already;
    let shift: Option<String> =
        sqlx::query_scalar("SELECT id FROM shifts ORDER BY (closed_at IS NULL) DESC, opened_at DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let applicable = matches!(ostatus.as_str(), "confirmed" | "ready") && amount <= remaining && shift.is_some();
    if !applicable {
        // Money arrived but the order moved on (voided, already paid, changed): keep it visible for a human.
        sqlx::query("UPDATE gateway_charges SET status = 'unapplied', settled_at = ? WHERE id = ?")
            .bind(clock::now())
            .bind(&cid)
            .execute(&mut *tx)
            .await?;
        let mut e = Entry::new(None, "gateway.unapplied", "order", Some(&order_id));
        let d = format!("{provider_order_id} {amount}");
        e.detail = Some(&d);
        audit::record(&mut tx, e).await?;
        tx.commit().await?;
        return Ok(Outcome::Unapplied);
    }
    let shift = shift.expect("checked above");
    let fully = orders::apply_payment(
        &mut tx,
        orders::PaymentIn {
            order_id: &order_id,
            shift_id: &shift,
            method: "qris",
            amount,
            remaining,
            tendered: None,
            reference: Some(provider_order_id),
            cashier_id: &by,
        },
    )
    .await?;
    sqlx::query("UPDATE gateway_charges SET status = 'settled', settled_at = ? WHERE id = ?")
        .bind(clock::now())
        .bind(&cid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    orders::notify(st);
    if fully {
        super::receipt::auto_print(st.clone(), order_id.clone());
        emit_order_paid(st.clone(), order_id);
    }
    Ok(Outcome::Applied)
}

/// Provider -> Cuppa. Authenticated only by the signature; always answers quickly and never reveals why it refused.
pub async fn webhook_midtrans(State(st): State<AppState>, body: Bytes) -> AppResult<Response> {
    let Some(key) = gateway_key(&st).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let v: Value = serde_json::from_slice(&body).map_err(|_| AppError::Invalid("bad payload".into()))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (order_id, status_code, gross, sig) = (s("order_id"), s("status_code"), s("gross_amount"), s("signature_key"));
    if !ct_eq(
        &midtrans_signature(&order_id, &status_code, &gross, &key),
        &sig.to_lowercase(),
    ) {
        tracing::warn!(order_id = %order_id, "gateway webhook with a bad signature");
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    let out = settle(
        &st,
        &order_id,
        &s("transaction_status"),
        &s("fraud_status"),
        parse_amount(&gross),
    )
    .await?;
    Ok((
        if out == Outcome::Unknown {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        },
        "ok",
    )
        .into_response())
}

/// Reconciliation: ask the provider for the current status of one charge (FR-41).
pub async fn check_charge(State(st): State<AppState>, staff: Staff, Path(cid): Path<String>) -> AppResult<Response> {
    staff.require(&[Role::Owner, Role::Cashier])?;
    let (poid, order_id): (String, String) =
        sqlx::query_as("SELECT provider_order_id, order_id FROM gateway_charges WHERE id = ?")
            .bind(&cid)
            .fetch_optional(&st.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let key = gateway_key(&st)
        .await?
        .ok_or_else(|| AppError::Invalid("Gateway belum diatur.".into()))?;
    let resp = http()
        .get(format!(
            "{}/v2/{}/status",
            st.cfg.midtrans_base.trim_end_matches('/'),
            poid
        ))
        .basic_auth(&key, Some(""))
        .send()
        .await;
    let Ok(r) = resp else {
        return orders::render_detail(
            &st,
            &staff,
            &order_id,
            Some("Gateway tidak dapat dihubungi.".into()),
            None,
        )
        .await;
    };
    let v: Value = r.json().await.unwrap_or(Value::Null);
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let out = settle(
        &st,
        &poid,
        &s("transaction_status"),
        &s("fraud_status"),
        parse_amount(&s("gross_amount")),
    )
    .await?;
    let msg = match out {
        Outcome::Applied => "Pembayaran QRIS diterima.",
        Outcome::AlreadyDone => "Pembayaran ini sudah diproses.",
        Outcome::Pending => "Belum dibayar. Minta pelanggan memindai kode.",
        Outcome::Closed => "QRIS kedaluwarsa atau ditolak. Buat QRIS baru.",
        Outcome::Unapplied => "Dana diterima tetapi pesanan sudah berubah. Periksa log audit.",
        Outcome::AmountMismatch => "Nominal tidak cocok. Periksa log audit.",
        Outcome::Unknown => "Transaksi tidak dikenal.",
    };
    orders::render_detail(&st, &staff, &order_id, None, Some(msg.into())).await
}

// ---- Outgoing webhooks (FR-62) -----------------------------------------------------------------

/// https only, a real hostname or public IP: the guard against turning this into an internal-network scanner.
/// Split `host[:port]` (including `[v6]:port`) out of the authority part of an https URL.
fn split_authority(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("https://")?;
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    if auth.contains('@') {
        return None; // credentials in the URL are never needed and are a classic parser-confusion trick
    }
    let (host, port) = if let Some(inner) = auth.strip_prefix('[') {
        let (h, after) = inner.split_once(']')?;
        (
            h.to_string(),
            after.strip_prefix(':').map(str::parse).transpose().ok()?.unwrap_or(443),
        )
    } else if let Some((h, p)) = auth.rsplit_once(':') {
        (h.to_string(), p.parse().ok()?)
    } else {
        (auth.to_string(), 443)
    };
    (!host.is_empty()).then_some((host, port))
}

pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Webhook harus memakai https://".into());
    }
    let (host_only, _) = split_authority(url).ok_or("Alamat webhook tidak valid.")?;
    let host_only = host_only.as_str();
    if let Ok(ip) = host_only.parse::<IpAddr>()
        && !is_public_ip(ip)
    {
        return Err("Webhook tidak boleh menuju alamat jaringan lokal.".into());
    }
    if host_only.eq_ignore_ascii_case("localhost") || host_only.ends_with(".local") || host_only.ends_with(".internal")
    {
        return Err("Webhook tidak boleh menuju alamat lokal.".into());
    }
    Ok(())
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_documentation()
                || v.octets()[0] == 0
                || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1])))
        }
        IpAddr::V6(v) => {
            !(v.is_loopback()
                || v.is_unspecified()
                || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Fire-and-forget `order.paid` event, signed with HMAC-SHA256. The destination is resolved once and pinned so a DNS answer
/// that changes between check and connect cannot redirect the request inward.
pub fn emit_order_paid(st: AppState, order_id: String) {
    tokio::spawn(async move {
        let Ok((Some(url), Some(enc))) = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT webhook_url, webhook_secret_enc FROM outlet_settings WHERE id = 1",
        )
        .fetch_one(&st.db)
        .await
        else {
            return;
        };
        let Some(secret) = secrets::decrypt(&st.secret_key, &enc) else {
            return;
        };
        if validate_webhook_url(&url).is_err() {
            return;
        }
        let row: Option<(i64, String, i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT number, day, subtotal, tax, total, COALESCE(paid_at, created_at) FROM orders WHERE id = ?",
        )
        .bind(&order_id)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
        let Some((number, day, subtotal, tax, total, at)) = row else {
            return;
        };
        let payload = json!({"event": "order.paid", "id": order_id, "number": number, "day": day, "subtotal": subtotal, "tax": tax, "total": total, "currency": "IDR", "paid_at": at});
        let body = payload.to_string();
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).expect("any key length");
        mac.update(body.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        let rest = url.trim_start_matches("https://");
        let hostport = rest.split(['/', '?', '#']).next().unwrap_or("");
        let (host, port) = hostport
            .rsplit_once(':')
            .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h, p)))
            .unwrap_or((hostport, 443));
        let Ok(addrs) = tokio::net::lookup_host((host, port)).await else {
            return;
        };
        let Some(addr) = addrs.into_iter().find(|a| is_public_ip(a.ip())) else {
            tracing::warn!("webhook destination does not resolve to a public address");
            return;
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .resolve(host, addr)
            .build();
        let Ok(client) = client else { return };
        if let Err(e) = client
            .post(&url)
            .header("content-type", "application/json")
            .header("x-cuppa-signature", format!("sha256={sig}"))
            .body(body)
            .send()
            .await
        {
            tracing::warn!(error = ?e, "outgoing webhook failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midtrans_signature_known_vector() {
        // sha512("order-1" "200" "10000.00" "server-key")
        let s = midtrans_signature("order-1", "200", "10000.00", "server-key");
        assert_eq!(s.len(), 128);
        assert_eq!(s, hex::encode(Sha512::digest(b"order-120010000.00server-key")));
    }

    #[test]
    fn amounts_reject_fractions() {
        assert_eq!(parse_amount("55000.00"), Some(55_000));
        assert_eq!(parse_amount("55000"), Some(55_000));
        assert_eq!(parse_amount("55000.50"), None);
        assert_eq!(parse_amount("-1"), None);
        assert_eq!(parse_amount(""), None);
    }

    #[test]
    fn webhook_url_guard() {
        assert!(validate_webhook_url("https://hooks.example.com/cuppa").is_ok());
        assert!(validate_webhook_url("http://hooks.example.com").is_err());
        for bad in [
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://10.0.0.5/x",
            "https://192.168.1.1",
            "https://169.254.169.254/latest",
            "https://[::1]/x",
            "https://user@evil.com@127.0.0.1/",
            "https://printer.local/",
        ] {
            assert!(validate_webhook_url(bad).is_err(), "{bad}");
        }
    }
}
