# PRD coverage

Honest status of every PRD requirement. ✅ done and tested · 🟡 partial · ⏳ planned (not in v0.1).

## Core (FR-01 – FR-40)
| ID | Status | Notes |
| --- | --- | --- |
| FR-01 | ✅ | Token URL, no login, café name, open/closed banner (opening hours). |
| FR-02 | ✅ | Categories, photos (re-encoded, lazy-loaded), prices, sold-out/low-stock labels, variants. |
| FR-03 | ✅ | Required/optional modifier groups with max selection; validated and priced on the server. |
| FR-04 | 🟡 | Live subtotal + note; exact tax/service breakdown is computed and shown after submit. |
| FR-05 | ✅ | Server re-prices; rejects sold-out/inactive items, inactive table, outside hours, oversell. |
| FR-06 | ✅ | Live status page (polling every 5 s). |
| FR-07 | ✅ | Additional orders at the same table + a table bill page listing every order of the day. |
| FR-08 | ✅ | Combined queue, SSE refresh, beep on new QR order. |
| FR-09 | ✅ | Confirm/reject with reason; auto-cancel after N minutes (stock restored). |
| FR-10 | ✅ | Manual order, optional table, daily queue number. |
| FR-11 | ✅ | Void items with reason before payment (totals recalculated), move table. |
| FR-12 | ✅ | Order discounts (% or Rp), cashier limit, owner approval above it, audited. |
| FR-13 | ✅ | Cash (quick chips, change), QRIS/EDC/transfer with reference. |
| FR-14 | ✅ | Receipt: browser print + ESC/POS LAN; reprint is possible any time. Not verified on real hardware. |
| FR-15 | ✅ | |
| FR-16 | ✅ | Partial/full refund as separate immutable document + approval; cash refunds leave the drawer. |
| FR-17 | ✅ | Cash in/out and open-drawer with reason. |
| FR-18 | ✅ | Blind close, Z report incl. cash in/out. |
| FR-19 | ✅ | Order history with search by number/table and date. |
| FR-20 | ✅ | Stations mapped to categories, kitchen screen filter per station, tickets printed to each station's printer. |
| FR-21 | ✅ | Kitchen screen + ticket printing (kitchen mode). |
| FR-22 | ✅ | Sold-out toggle for cashier/kitchen. |
| FR-23 | ✅ | CRUD, deactivate, CSV import, photo upload, stock, cost. |
| FR-24 | ✅ | Optional stock per item, auto sold-out at 0, restored on cancel. |
| FR-25 | ✅ | Tables, printable QR, token rotation. |
| FR-26 | ✅ | Tax, service, rounding (own line), receipt footer, opening hours. |
| FR-27 | ✅ | First-run wizard for the four QR × kitchen combinations; unused pages hidden. |
| FR-28 | ✅ | Dashboard + reports (sales, menu, method, cashier, shift differences). |
| FR-29 | ✅ | CSV export. |
| FR-30 | ✅ | Customer-page accent theme presets. |
| FR-31 | ✅ | Role defaults + per-user grant/revoke, enforced server-side. |
| FR-32 | ✅ | PIN login restricted to registered devices (revocable); owner uses a passphrase. |
| FR-33 | ✅ | Approver is a different owner (or owner re-auth), 5-strikes lockout, idle screen lock (10 min). |
| FR-34 | ✅ | Append-only audit log, DB-enforced paid lock. |
| FR-35 | ✅ | SQLite WAL default. |
| FR-36 | ⏳ | Postgres dialect: see ADR 0001. |
| FR-37 | ✅ | Daily + manual backup, `verify-backup`, `scripts/restore.sh`. |
| FR-38 | ✅ | Automatic pre-migration snapshot. |
| FR-39 | 🟡 | ESC/POS LAN + drawer + test print implemented; needs a real-printer check. |
| FR-40 | ✅ | Docker, systemd, nginx sample, install guide. |

## Additional (FR-41 – FR-66)
| ID | Status | Notes |
| --- | --- | --- |
| FR-41–42 | 🟡 | Midtrans QRIS: dynamic QR per order, HMAC-SHA512 signed idempotent webhook, amount check, manual status check/reconciliation, orphaned money flagged for review. Tested against a local mock of the API only — **not yet against a live Midtrans account**. Xendit and gateway refunds are not built (refunds stay manual). |
| FR-43 | ✅ | Split / mixed payment. |
| FR-44 | ✅ | Ingredients, recipes, purchases with moving-average cost, opname, waste, immutable movements, actual cost per order. Suppliers are not modelled. |
| FR-45 | ✅ | Expenses, per-menu cost (manual or recipe), gross and net profit. |
| FR-46, 60, 66 | ⏳ | Multi-branch / multi-tenant: needs the Postgres backend and a tenant model. |
| FR-47, 58 | ✅ | Rating per order and per menu item. |
| FR-48 | ✅ | Busiest hours, margin per menu, automatic busy mode (threshold + extra wait). |
| FR-49 | ✅ | Vouchers/promos and time-of-day (happy hour) prices, snapshotted on the order line. |
| FR-50 | ✅ | Custom CSS (allowlisted properties, values and selectors, always scoped to the customer page, re-sanitised on every serve) and a sanitised HTML banner (text, lists and https links only). Cuppa never executes owner code. |
| FR-56 | ⏳ | Custom JS in a sandboxed iframe on a separate origin: deliberately not built until an independent security review (the PRD itself requires it). |
| FR-51 | 🟡 | Owner 2FA (TOTP, replay-protected). Custom roles are covered by per-user permission overrides. |
| FR-52 | ✅ | Public digital receipt link + WhatsApp share link (no WhatsApp API sending). |
| FR-53 | ✅ | Floor plan, merge orders, move table, call waiter, ask for the bill, Indonesian/English switch for every customer page (text nodes only). Staff screens are Indonesian. |
| FR-54 | ⏳ | SQLite → Postgres migration tool (depends on FR-36). |
| FR-55 | ✅ | Delivery channels as order source (manual). |
| FR-57 | ✅ | Opt-in loyalty with consent, points, favourite item and erasure. |
| FR-59 | ⏳ | GoFood/GrabFood/ShopeeFood integration: needs partner access. |
| FR-61 | ✅ | Reservations (deposit is a note, not accounted money). |
| FR-62 | ✅ | Customer queue display, read-only API with hashed revocable keys, signed `order.paid` webhooks (https only, SSRF-guarded, DNS pinned). The outgoing webhook was unit-tested for its URL guard but not against a live receiver. |
| FR-63 | 🟡 | Installable PWA with a service worker (public assets + offline notice; authenticated pages are never cached). The server runs on the café LAN, so cashier and kitchen keep working without internet. Offline transactions while the server itself is unreachable (with conflict resolution) are not built. |
| FR-64 | 🟡 | Installable PWA (manifest + icons) for Android/PC. Native iOS/Android/desktop wrappers with direct printer access are not built. |
| FR-65 | ✅ | Balanced in-app ledger, journal CSV, local-tax CSV. |
| FR-36 | ⏳ | Postgres. The data layer uses SQLite-specific SQL and triggers; a second dialect is a project of its own (ADR 0001). |

Receipt printing and the cash drawer are implemented but have not been exercised on real hardware.
