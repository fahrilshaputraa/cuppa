# Changelog

Format: [Keep a Changelog](https://keepachangelog.com), versioning: [SemVer](https://semver.org).

## [Unreleased]

### Added
- Foundation: Axum + SQLx (SQLite WAL), embedded migrations, Askama + HTMX UI, SSE.
- First-run setup code, Argon2id PIN/passphrase login, lockout, server-side sessions.
- Menu, tables + QR, staff, settings (tax/service), append-only audit log.
- QR customer ordering with server-side pricing; cashier queue, payments, shifts with blind close, void with second-owner approval.
- DESIGN.md design system, CI (fmt, clippy, test, cargo-deny, RustSec), Dockerfile.
