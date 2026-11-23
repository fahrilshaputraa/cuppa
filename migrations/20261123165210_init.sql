-- Cuppa initial schema (SQLite). Money = integer rupiah, time = UTC unix seconds, ids = TEXT.

CREATE TABLE outlet_settings (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    outlet_name        TEXT    NOT NULL DEFAULT 'Cuppa Cafe',
    tax_bp             INTEGER NOT NULL DEFAULT 1000 CHECK (tax_bp BETWEEN 0 AND 10000),      -- basis points
    service_bp         INTEGER NOT NULL DEFAULT 0    CHECK (service_bp BETWEEN 0 AND 10000),
    tax_inclusive      INTEGER NOT NULL DEFAULT 0    CHECK (tax_inclusive IN (0,1)),
    service_in_tax_base INTEGER NOT NULL DEFAULT 1   CHECK (service_in_tax_base IN (0,1)),
    qr_ordering        INTEGER NOT NULL DEFAULT 1    CHECK (qr_ordering IN (0,1)),
    confirm_timeout_min INTEGER NOT NULL DEFAULT 10  CHECK (confirm_timeout_min > 0)
);
INSERT INTO outlet_settings (id) VALUES (1);

CREATE TABLE users (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    role            TEXT NOT NULL CHECK (role IN ('owner','cashier','kitchen')),
    pin_hash        TEXT NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until    INTEGER NOT NULL DEFAULT 0,
    active          INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
    created_at      INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,           -- sha256(token); raw token only lives in the cookie
    user_id    TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);

CREATE TABLE categories (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    sort_order INTEGER NOT NULL DEFAULT 0,
    active     INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);

CREATE TABLE menu_items (
    id          TEXT PRIMARY KEY,
    category_id TEXT NOT NULL REFERENCES categories(id),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    price       INTEGER NOT NULL CHECK (price >= 0),
    active      INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),   -- never hard-deleted
    available   INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0,1)) -- sold-out toggle
);

CREATE TABLE tables (
    id       TEXT PRIMARY KEY,
    label    TEXT NOT NULL UNIQUE CHECK (length(label) BETWEEN 1 AND 20),
    token    TEXT NOT NULL UNIQUE,   -- 256-bit random, rotatable
    active   INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);

CREATE TABLE shifts (
    id             TEXT PRIMARY KEY,
    opened_by      TEXT NOT NULL REFERENCES users(id),
    opening_cash   INTEGER NOT NULL CHECK (opening_cash >= 0),
    opened_at      INTEGER NOT NULL,
    closed_at      INTEGER,
    counted_cash   INTEGER,
    expected_cash  INTEGER,
    difference     INTEGER
);
CREATE UNIQUE INDEX idx_one_open_shift ON shifts((1)) WHERE closed_at IS NULL;

CREATE TABLE daily_counters (day TEXT PRIMARY KEY, n INTEGER NOT NULL);

CREATE TABLE orders (
    id            TEXT PRIMARY KEY,
    day           TEXT NOT NULL,
    number        INTEGER NOT NULL,
    table_id      TEXT REFERENCES tables(id),
    source        TEXT NOT NULL CHECK (source IN ('qr','cashier')),
    status        TEXT NOT NULL CHECK (status IN ('pending','confirmed','ready','paid','cancelled','voided')),
    subtotal      INTEGER NOT NULL,
    service       INTEGER NOT NULL,
    tax           INTEGER NOT NULL,
    total         INTEGER NOT NULL,
    note          TEXT,
    cashier_id    TEXT REFERENCES users(id),
    cancel_reason TEXT,
    created_at    INTEGER NOT NULL,
    paid_at       INTEGER,
    UNIQUE (day, number)
);
CREATE INDEX idx_orders_status ON orders(status, created_at);

CREATE TABLE order_items (
    id         TEXT PRIMARY KEY,
    order_id   TEXT NOT NULL REFERENCES orders(id),
    menu_id    TEXT NOT NULL REFERENCES menu_items(id),
    name       TEXT NOT NULL,         -- copied at order time
    unit_price INTEGER NOT NULL,      -- copied at order time
    qty        INTEGER NOT NULL CHECK (qty BETWEEN 1 AND 99),
    note       TEXT
);
CREATE INDEX idx_items_order ON order_items(order_id);

CREATE TABLE payments (
    id         TEXT PRIMARY KEY,
    order_id   TEXT NOT NULL REFERENCES orders(id),
    shift_id   TEXT NOT NULL REFERENCES shifts(id),
    method     TEXT NOT NULL CHECK (method IN ('cash','qris','card','transfer')),
    amount     INTEGER NOT NULL CHECK (amount > 0),   -- order total settled
    tendered   INTEGER,
    reference  TEXT,                                   -- never a full card number
    cashier_id TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    actor_id   TEXT,
    approver_id TEXT,
    action     TEXT NOT NULL,
    entity     TEXT NOT NULL,
    entity_id  TEXT,
    detail     TEXT,
    reason     TEXT
);

-- ---- Integrity rules enforced by the database itself ----
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_log BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

-- A paid order is locked: the only allowed transition is paid -> voided (done by the app with supervisor approval + audit).
CREATE TRIGGER orders_paid_lock BEFORE UPDATE ON orders
WHEN OLD.status IN ('paid','voided') AND NOT (OLD.status = 'paid' AND NEW.status = 'voided'
     AND NEW.total = OLD.total AND NEW.subtotal = OLD.subtotal)
BEGIN SELECT RAISE(ABORT, 'order is locked after payment'); END;
CREATE TRIGGER orders_no_delete BEFORE DELETE ON orders BEGIN SELECT RAISE(ABORT, 'orders cannot be deleted'); END;

CREATE TRIGGER items_paid_lock_u BEFORE UPDATE ON order_items
WHEN (SELECT status FROM orders WHERE id = OLD.order_id) IN ('paid','voided')
BEGIN SELECT RAISE(ABORT, 'order is locked after payment'); END;
CREATE TRIGGER items_paid_lock_d BEFORE DELETE ON order_items
BEGIN SELECT RAISE(ABORT, 'order items cannot be deleted'); END;
CREATE TRIGGER items_paid_lock_i BEFORE INSERT ON order_items
WHEN (SELECT status FROM orders WHERE id = NEW.order_id) IN ('paid','voided','cancelled')
BEGIN SELECT RAISE(ABORT, 'order is locked'); END;

CREATE TRIGGER payments_no_update BEFORE UPDATE ON payments BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;
CREATE TRIGGER payments_no_delete BEFORE DELETE ON payments BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;
