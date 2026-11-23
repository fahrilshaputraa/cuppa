-- PRD extended: channels, digital receipt link, vouchers, ratings, table calls, expenses and per-item cost (COGS).

ALTER TABLE menu_items ADD COLUMN cost INTEGER NOT NULL DEFAULT 0 CHECK (cost >= 0);  -- COGS per unit (simple recipe cost)
ALTER TABLE order_items ADD COLUMN unit_cost INTEGER NOT NULL DEFAULT 0;        -- snapshot at order time

ALTER TABLE orders ADD COLUMN channel TEXT NOT NULL DEFAULT 'dine_in' CHECK (channel IN ('dine_in','takeaway','gofood','grabfood','shopeefood'));
ALTER TABLE orders ADD COLUMN receipt_token TEXT;                               -- public digital receipt link

-- Vouchers / promos (FR-49)
CREATE TABLE vouchers (
    id           TEXT PRIMARY KEY,
    code         TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (length(code) BETWEEN 3 AND 20),
    kind         TEXT NOT NULL CHECK (kind IN ('pct','rp')),
    value        INTEGER NOT NULL CHECK (value > 0),
    min_subtotal INTEGER NOT NULL DEFAULT 0,
    max_uses     INTEGER,
    used         INTEGER NOT NULL DEFAULT 0,
    valid_until  INTEGER,
    active       INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);

-- Ratings: one per order (FR-47)
CREATE TABLE order_ratings (
    order_id   TEXT PRIMARY KEY REFERENCES orders(id),
    stars      INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    comment    TEXT,
    created_at INTEGER NOT NULL
);

-- Call waiter / ask for the bill (FR-53)
CREATE TABLE table_calls (
    id         TEXT PRIMARY KEY,
    table_id   TEXT NOT NULL REFERENCES tables(id),
    kind       TEXT NOT NULL CHECK (kind IN ('waiter','bill')),
    created_at INTEGER NOT NULL,
    handled_at INTEGER,
    handled_by TEXT REFERENCES users(id)
);
CREATE INDEX idx_calls_open ON table_calls(handled_at, created_at);

-- Operating expenses for profit reports (FR-45)
CREATE TABLE expenses (
    id         TEXT PRIMARY KEY,
    day        TEXT NOT NULL,
    category   TEXT NOT NULL CHECK (length(category) BETWEEN 1 AND 40),
    amount     INTEGER NOT NULL CHECK (amount > 0),
    kind       TEXT NOT NULL CHECK (kind IN ('fixed','variable')),
    note       TEXT,
    user_id    TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_expenses_day ON expenses(day);
