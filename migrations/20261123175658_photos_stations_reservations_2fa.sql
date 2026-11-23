-- Photos (FR-02/23), stations (FR-20), reservations (FR-61), opt-in customers (FR-57), owner 2FA (FR-51).

ALTER TABLE menu_items ADD COLUMN photo TEXT;                 -- random file name; image is re-encoded server-side

CREATE TABLE stations (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 30),
    printer_host TEXT
);
ALTER TABLE categories ADD COLUMN station_id TEXT REFERENCES stations(id);

CREATE TABLE reservations (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    phone      TEXT,
    at_ts      INTEGER NOT NULL,
    pax        INTEGER NOT NULL CHECK (pax BETWEEN 1 AND 200),
    deposit    INTEGER NOT NULL DEFAULT 0 CHECK (deposit >= 0),
    status     TEXT NOT NULL DEFAULT 'booked' CHECK (status IN ('booked','arrived','cancelled')),
    note       TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_resv_at ON reservations(at_ts);

-- Only created when the customer ticks the consent box; erasable by the owner.
CREATE TABLE customers (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    phone      TEXT NOT NULL UNIQUE,
    consent_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
-- Paid orders are frozen by trigger, so the optional link lives in its own table.
CREATE TABLE order_customers (
    order_id    TEXT PRIMARY KEY REFERENCES orders(id),
    customer_id TEXT NOT NULL REFERENCES customers(id)
);
CREATE INDEX idx_order_customers_c ON order_customers(customer_id);

ALTER TABLE users ADD COLUMN totp_secret TEXT;                -- hex of the 20-byte shared secret, NULL = 2FA off
ALTER TABLE users ADD COLUMN totp_pending TEXT;               -- generated, not yet confirmed
ALTER TABLE users ADD COLUMN totp_last INTEGER NOT NULL DEFAULT 0;   -- last accepted time step (replay guard)
