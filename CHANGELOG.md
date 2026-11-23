# Changelog

Format: [Keep a Changelog](https://keepachangelog.com), versioning: [SemVer](https://semver.org).

## [Unreleased]

### Added
- Foundation: Axum + SQLx (SQLite WAL), embedded migrations, Askama + HTMX UI, SSE.
- First-run setup code, Argon2id PIN/passphrase login, lockout, server-side sessions.
- Menu, tables + QR, staff, settings (tax/service), append-only audit log.
- QR customer ordering with server-side pricing; cashier queue, payments, shifts with blind close, void with second-owner approval.
- Receipts (printable HTML + ESC/POS over LAN, cash drawer pulse, LAN-only printer guard), owner reports + CSV export (formula-injection safe), daily rotating backups + audited download.
- UX: quantity steppers with live subtotal, quick-cash chips with live change, grouped navigation, mobile bottom nav.
- PRD core: wizard (flow modes), modifiers, stock, discounts with approval, item void, table move, refunds, cash in/out, history, CSV import, opening hours, rounding, themes, per-user permissions, registered devices, idle lock, kitchen ticket printing, new-order sound.
- Redesign (unslop): self-hosted Figtree, neutral ramp + one accent, line icons, perforated tickets, thermal receipt, sales chart.
- Split/mixed payment, vouchers, digital receipt link, call waiter / bill, ratings, queue display, expenses + profit, menu photos, stations, reservations, loyalty, owner 2FA.
- Recipes and ingredient stock (purchases, opname, waste), per-menu ratings, busy mode, floor plan, order merge, read-only API keys, in-app ledger.
- Installable PWA (manifest, icons, service worker), Midtrans QRIS gateway with signed idempotent webhooks, encrypted credentials, signed outgoing webhooks, scheduled prices.
- Safe operations: snapshot before migrations, `verify-backup`, restore script, install guide, nginx/systemd samples, `docs/PRD-COVERAGE.md`.
- DESIGN.md design system, CI (fmt, clippy, test, cargo-deny, RustSec), Dockerfile.
