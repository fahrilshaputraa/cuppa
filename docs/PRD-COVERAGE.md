# PRD coverage

Honest status of every PRD requirement. ✅ done and tested · 🟡 partial · ⏳ planned (not in v0.1).

## Core (FR-01 – FR-40)
| ID | Status | Notes |
| --- | --- | --- |
| FR-01 | ✅ | Token URL, no login, café name, open/closed banner (opening hours). |
| FR-02 | 🟡 | Categories, prices, sold-out/low-stock labels, variants. No photos yet. |
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
| FR-20 | 🟡 | Kitchen mode + separate kitchen printer; per-category stations not built. |
| FR-21 | ✅ | Kitchen screen + ticket printing (kitchen mode). |
| FR-22 | ✅ | Sold-out toggle for cashier/kitchen. |
| FR-23 | 🟡 | CRUD, deactivate, CSV import. No photo upload. |
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

## Additional (FR-41+)
Built: FR-43 split/mixed payment, FR-45 (expenses + per-item cost → gross/net profit), FR-47 order rating, FR-49 vouchers (scheduled prices not built), FR-52 digital receipt link with WhatsApp share, FR-53 call waiter / ask for bill / table bill (merge-split tables and floor plan not built), FR-55 manual delivery channels (GoFood/GrabFood/ShopeeFood as source), FR-62 customer queue display (staff API/webhooks not built).

Still not built:
payment gateway (41–42), ingredient stock/recipes (44), multi-branch (46, 60, 66), per-menu rating (58), advanced analytics (48), custom CSS/JS (50, 56 — needs a dedicated security review), 2FA/custom roles (51), floor plan/merge tables/translation (53), Postgres + migration tool (36, 54), loyalty/customers (57), platform integrations (59), reservations (61), API/webhooks (62), full offline mode (63), native wrappers (64), accounting (65).
