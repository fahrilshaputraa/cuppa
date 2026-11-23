# Contributing

Thanks for helping! Code, comments and docs are in **English**; the UI copy is Indonesian.

1. Open an issue first for anything non-trivial.
2. `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` must pass.
3. New endpoints: check the role server-side **and** add a test (see `tests/flow.rs`).
4. Money is integer rupiah; schema changes go through `sqlx migrate add`, never edit applied migrations.
5. UI follows [DESIGN.md](DESIGN.md) tokens; no inline scripts/styles (CSP) and no third-party assets.
6. Every file starts with `// SPDX-License-Identifier: AGPL-3.0-or-later`.
7. **DCO:** sign off each commit (`git commit -s`). Use Conventional Commits (`feat:`, `fix:`, `docs:`…).
8. Never commit secrets, `.env`, or real customer data.
