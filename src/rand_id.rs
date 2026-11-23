// SPDX-License-Identifier: AGPL-3.0-or-later
//! OS-backed random identifiers and tokens.

/// `n` random bytes, hex encoded. Panics only if the OS RNG is unavailable (unrecoverable).
pub fn hex_token(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("OS random source unavailable");
    hex::encode(buf)
}

pub fn id() -> String {
    hex_token(16)
}
