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

- **Cashier** — live queue (QR + manual orders), confirm/reject, cash/QRIS/card/transfer, change calculation, blind shift close with Z report.
- **Customer QR ordering** — scan the table QR, no app, no login. Prices are always recomputed on the server.
- **Owner** — menu & categories (never hard-deleted), tables + printable QR, tax/service settings, staff, audit log.
- **Kitchen** — ticket screen with "ready" action.
- **Money rules** — integer rupiah, price copied into order lines, paid orders locked by the database, void needs a *second owner's* credentials + reason.
- **Realtime** — Server-Sent Events, no websockets, no JS framework (HTMX).

## Security by default

| Area | Measure |
| --- | --- |
| Secrets | Argon2id; lockout after 5 failures; constant-time-ish unknown-user path |
| Sessions | Random 256-bit token, only its SHA-256 stored; `HttpOnly; SameSite=Strict` (+`Secure` over HTTPS) |
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

Stack: Axum · SQLx (SQLite WAL) · Askama · HTMX · SSE. Design system: [DESIGN.md](DESIGN.md). Product docs: [PRD](docs/PRD.md), [Spec](docs/SPEC.md), [ADRs](docs/adr).

## Roadmap

Built: foundation, master data, QR ordering, cash/non-cash payment, shifts, void with approval, audit.
Next (PRD order): ESC/POS receipt & drawer, sales/tax reports + CSV, backups & restore test, per-user permission overrides, modifiers, Postgres, split bill, payment gateway, stock & recipes, multi-branch.

## License

[AGPL-3.0-or-later](LICENSE). The name and logo are not covered by the license. Contributions use the [DCO](CONTRIBUTING.md).
