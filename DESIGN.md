# Cuppa — Design System

Source of truth for every screen. Tokens live in `static/css/tokens.css`; never hardcode a value that has a token.

## How this was derived

References pulled with the `design-pulse` MCP (playbook `ui-build-workflow`, `dashboard-design-checklist`,
`Where Claude's default UI goes wrong`, plus the `data-table` / `dashboard` feed).

**Borrowed**
- *Answer first, density with rhythm* — the 1–3 numbers a user opens the screen for sit top-left and largest;
  rows are 44px; numbers use tabular figures so digits never jitter (design-pulse dashboard checklist; same
  restraint as shadcn's data-table: neutral surface, one accent, hairline borders).
- *Tokens before components*, one radius, one elevation method (borders, not shadows).
- *States are part of done*: empty / loading / error / long text on every list.

**Refused**
- The indigo/violet default, gray-50 canvas and "card for everything". Cuppa's palette comes from its own world: paper, espresso, crema, copper.
- Emoji as icons. Icons are inline SVG (Lucide-style, 1.75 stroke).

## Modes

| Surface | Mode | Priority |
| --- | --- | --- |
| Cashier (tablet, one hand) | Operate | speed, 44px+ targets, no scrolling for the core flow |
| Owner back-office | Operate | scanability, tables, reports |
| Customer (phone, QR) | Experience | content (menu) leads, one clear action, < 300 KB |
| Kitchen | Operate | glanceable, large type, sound |

## Color

Warm neutrals + one accent. Light default, dark is a re-tuned palette (not inverted).

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--paper` | `#F7F3EC` | `#17120E` | page canvas |
| `--surface` | `#FFFFFF` | `#201A15` | panels, inputs |
| `--surface-2` | `#EFE9DF` | `#2A231C` | subtle fills, hover |
| `--line` | `#E1D8C9` | `#3A3128` | hairline borders |
| `--ink` | `#221A14` | `#F3ECE1` | primary text (≥ 12:1) |
| `--ink-2` | `#5F5347` | `#BFB3A3` | secondary text (≥ 5:1) |
| `--accent` | `#A8481F` | `#E0773F` | primary action, focus ring, selection |
| `--accent-ink` | `#FFFFFF` | `#1A0F08` | text on accent |
| `--ok` | `#2E6B4B` | `#6DBB93` | paid, ready |
| `--warn` | `#9A6200` | `#E3A84B` | waiting confirmation |
| `--danger` | `#B3261E` | `#F08A82` | void, errors |

Status is never colour-only: every status chip also carries a text label.

## Type

System UI stack (no third-party font requests: self-hosted, offline-capable, strict CSP).
Scale ratio 1.2 on a 16px base: `12 / 14 / 16 / 19 / 23 / 28 / 34`.
Money and counts: `font-variant-numeric: tabular-nums`. Page titles 700, labels 600, body 400.

## Space, shape, motion

- Spacing on a 4px grid: `4 8 12 16 24 32 48`.
- One radius: `--radius: 10px` (chips: full round). One elevation: 1px `--line` border; overlays only get a shadow.
- Motion 120–180ms, ease-out, `transform`/`opacity` only; everything respects `prefers-reduced-motion`.
- Tap targets ≥ 44px. Focus ring: 2px `--accent` + 2px offset, always visible on keyboard focus.

## Components (in `static/css/app.css`)

`btn` (primary / secondary / danger / ghost), `field`, `chip` (status), `panel`, `table-ledger`, `ticket`
(order card), `stat` (number tile), `empty`, `toast`, `sidebar`/`bottom-nav`.

## Screens (Phase 1 target)

1. **Login** — PIN pad (large keys, 4–6 digits, lockout message), owner password mode.
2. **Cashier queue** — left: incoming QR orders (warn chip, confirm/reject), right: active order; sticky total bar.
3. **Pay** — quick cash chips (exact, 50k, 100k), change in the largest type on screen.
4. **Shift** — blind close: enter counted cash first; system figure revealed after.
5. **Owner** — today's sales stat row, menu table, tables + QR print sheet.
6. **Customer menu** — category chips, item rows with photo slot, sticky cart bar, server-priced summary.

## Accessibility baseline

WCAG 2.2 AA contrast, full keyboard path, `aria-live` on order status and toasts, visible focus,
`lang="id"`, forms with real labels, no information by colour alone.
