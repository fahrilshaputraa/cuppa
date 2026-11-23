<h1 align="center">cuppa</h1>
<p align="center"><b>Self-hosted POS and QR ordering for small cafés.</b><br>One Rust binary · SQLite · no per-month fee · secure by default.</p>

<p align="center">
  <a href="https://github.com/fahrilshaputraa/cuppa/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/fahrilshaputraa/cuppa/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0-blue">
  <img alt="Rust" src="https://img.shields.io/badge/rust-2024-orange">
</p>

> **Status: early development (v0.1).** The money path works end to end and is tested; many PRD items are still ahead (see [Roadmap](#roadmap)). Not yet validated in a real café.

## Why

Small cafés lose money in three places: cash leakage (fake voids and discounts, drawer mismatches), slow checkout at peak, and manual profit tracking. Hosted POS costs Rp150k–1M per outlet per month. Cuppa runs on a mini-PC in your café and keeps your data with you.

## Features (v0.1)

- **First-run wizard** — pick the flow: QR ordering on/off × "cashier only" / "up to the kitchen", local tax rate, device lock.
- **Customer QR ordering** — scan the table QR, no app, no login. Variants/toppings, live subtotal, open/closed hours. Prices are always recomputed on the server.
- **Cashier** — live queue with new-order sound, confirm/reject, manual orders, item void, table move, discounts (owner approval above the cashier limit), cash/QRIS/card/transfer, quick-cash chips with live change, history & search, cash in/out, blind shift close with Z report.
- **Refunds & voids** — partial/full refund documents and post-payment void, always with a reason and owner approval, fully audited.
- **Receipts** — browser print or ESC/POS LAN printer, cash-drawer pulse, kitchen/bar ticket printing, test-print button.
- **Owner** — menu (photos, variants, stock, cost, CSV import), kitchen/bar stations, reservations, opt-in loyalty customers, 2FA, tables + printable QR, staff with per-user permission overrides, registered devices, tax/service/rounding/hours/theme, reports (+CSV), audit log.
- **Back-office** — ingredients, recipes and stock movements with actual cost, expenses, profit, ledger and journal exports, read-only API keys, floor plan, order merge.
- **Data safety** — daily rotating backups, one-click download, `cuppa verify-backup`, automatic snapshot before migrations, restore script.
- **Money rules** — integer rupiah, price copied into order lines, rounding recorded as its own line, paid orders locked by the database.
- **Realtime** — Server-Sent Events + HTMX, no JS framework.

Full requirement-by-requirement status: [docs/PRD-COVERAGE.md](docs/PRD-COVERAGE.md).

## Security by default

| Area | Measure |
| --- | --- |
| Secrets | Argon2id; lockout after 5 failures; same Argon2 work for unknown users; optional owner 2FA (TOTP, replay-protected) |
| Uploads | Images decoded with size limits and re-encoded to JPEG; server-chosen random names; never served as-is |
| Sessions | Random 256-bit token, only its SHA-256 stored; `HttpOnly; SameSite=Strict` (+`Secure` over HTTPS); 12 h max, 10 min idle lock |
| Devices | Cashier/kitchen PINs work only on devices the owner registered (revocable) |
| First run | No default credentials. A one-time setup code is printed to the server log |
| Network | Binds to `127.0.0.1` by default; rate limits; proxy headers trusted only if `CUPPA_TRUST_PROXY=true` |
| Browser | Strict CSP (no inline script/style, no third-party origins), HSTS, frame denial, `no-store`, Origin-checked POSTs |
| Data integrity | SQLite triggers: append-only `audit_log`, immutable payments, locked paid orders |
| Authorization | Every handler checks the role on the server; covered by tests |
| Supply chain | `cargo-deny`, RustSec audit and Dependabot in CI; assets vendored (no CDN) |

Details and reporting: [SECURITY.md](SECURITY.md).

## Quick start

```bash
git clone https://github.com/fahrilshaputraa/cuppa && cd cuppa
cp .env.example .env            # optional
cargo run --release
# → watch the log for: FIRST RUN — open …/setup and enter setup code: xxxxxxxx
```

Open `http://localhost:8080/setup`, enter the code, create the owner. Then add menu, tables, and staff.

**Docker:** `docker compose up -d --build` (publishes on loopback only; put nginx/Caddy with HTTPS in front — see the sample in [docs/SPEC.md](docs/SPEC.md)).

### Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `CUPPA_BIND` | `127.0.0.1:8080` | Listen address |
| `CUPPA_PUBLIC_URL` | `http://localhost:8080` | Base URL in QR codes; `https://` enables Secure cookies + HSTS |
| `DATABASE_URL` | `sqlite://data/cuppa.db` | SQLite file (Postgres planned) |
| `CUPPA_TRUST_PROXY` | `false` | Trust last `X-Forwarded-For` entry |
| `CUPPA_TZ_OFFSET_HOURS` | `7` | Outlet timezone (WIB) |

## Development

```bash
cargo install sqlx-cli --no-default-features --features rustls,sqlite
sqlx migrate add <name>         # new migration (never edit applied ones)
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Stack: Axum · SQLx (SQLite WAL) · Askama · HTMX · SSE. Design system: [DESIGN.md](DESIGN.md). Product docs: [PRD](docs/PRD.md), [coverage](docs/PRD-COVERAGE.md), [install guide](docs/INSTALL.md), [Spec](docs/SPEC.md), [ADRs](docs/adr).

## Roadmap

Next: Postgres dialect, payment gateway (Midtrans/Xendit), split bill, ingredient stock & profit report, multi-branch, offline mode, photo upload, digital receipts. See the coverage table for details; real-printer verification is still pending.

## License

[AGPL-3.0-or-later](LICENSE). The name and logo are not covered by the license. Contributions use the [DCO](CONTRIBUTING.md).
