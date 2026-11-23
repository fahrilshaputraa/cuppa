# ADR 0001 — Stack and data layer

**Status:** accepted · 2026-11-23

- **Axum + HTMX + Askama + SSE**: server-rendered, one binary, no JS build, easy strict CSP.
- **SQLx with runtime-checked `query`/`query_as` (no compile-time macros)**: keeps the door open to Postgres without a compile-time DB dependency. SQL is kept portable (integer money, UTC unix time, TEXT ids, no JSON/array operators).
- **SQLite (WAL) first.** Migrations live in `migrations/` (SQLite dialect). A Postgres dialect directory + dual-backend CI is a roadmap item (FR-36); until then `DATABASE_URL` only accepts SQLite.
- **Integrity in the database:** triggers make `audit_log` append-only, `payments` immutable, and paid orders locked (only `paid → voided` allowed).
- **Write transactions use `BEGIN IMMEDIATE`** so concurrent writers queue instead of failing; daily order numbers come from an atomic upsert counter.
- **Security tradeoff:** a custom in-memory rate limiter instead of a heavy middleware dependency; fine for a single process, revisit for multi-instance.
