// SPDX-License-Identifier: AGPL-3.0-or-later
//! RFC 6238 TOTP (SHA-1, 6 digits, 30 s) for owner two-step login. Compatible with any authenticator app.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

const STEP: i64 = 30;

pub fn new_secret_hex() -> String {
    crate::rand_id::hex_token(20)
}

fn code_for(secret: &[u8], step: i64) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&(step as u64).to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin =
        ((h[off] as u32 & 0x7f) << 24) | ((h[off + 1] as u32) << 16) | ((h[off + 2] as u32) << 8) | h[off + 3] as u32;
    bin % 1_000_000
}

/// Returns the accepted time step if `code` is valid within ±1 step and newer than `last_step` (replay guard).
pub fn verify(secret_hex: &str, code: &str, now: i64, last_step: i64) -> Option<i64> {
    let secret = hex::decode(secret_hex).ok()?;
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let cur = now / STEP;
    // Always evaluate all three candidates so timing does not reveal which one matched.
    let mut hit = None;
    for s in [cur - 1, cur, cur + 1] {
        if code_for(&secret, s) == want && s > last_step {
            hit = Some(s);
        }
    }
    hit
}

pub fn base32(bytes: &[u8]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(A[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn otpauth_uri(account: &str, issuer: &str, secret_hex: &str) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    let key = base32(&hex::decode(secret_hex).unwrap_or_default());
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&digits=6&period=30",
        enc(issuer),
        enc(account),
        key,
        enc(issuer)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_test_vectors() {
        // RFC 6238 appendix B, secret "12345678901234567890", T=59 -> 94287082 (8 digits); last 6 digits = 287082.
        let secret = b"12345678901234567890";
        assert_eq!(code_for(secret, 59 / 30), 287_082);
        assert_eq!(code_for(secret, 1_111_111_109 / 30), 81_804);
    }

    #[test]
    fn verify_window_and_replay() {
        let hexs = hex::encode(b"12345678901234567890");
        let now = 59;
        assert_eq!(verify(&hexs, "287082", now, 0), Some(1));
        assert_eq!(verify(&hexs, "287082", now, 1), None, "same step cannot be reused");
        assert_eq!(verify(&hexs, "000000", now, 0), None);
        assert_eq!(verify(&hexs, "abc", now, 0), None);
    }

    #[test]
    fn base32_matches_known_value() {
        assert_eq!(base32(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }
}
