-- Per-menu ratings (FR-58), busy mode (FR-48), ingredients/recipes/stock movements (FR-44), read-only API keys (FR-62).

CREATE TABLE order_item_ratings (
    order_id TEXT NOT NULL REFERENCES orders(id),
    menu_id  TEXT NOT NULL REFERENCES menu_items(id),
    stars    INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    PRIMARY KEY (order_id, menu_id)
);

ALTER TABLE outlet_settings ADD COLUMN busy_threshold INTEGER NOT NULL DEFAULT 0 CHECK (busy_threshold >= 0);   -- 0 = off
ALTER TABLE outlet_settings ADD COLUMN busy_extra_min INTEGER NOT NULL DEFAULT 10 CHECK (busy_extra_min BETWEEN 0 AND 120);

-- Quantities are integers in a base unit (gram, ml, pcs). Costs are rupiah x 1000 per base unit to keep integer math.
CREATE TABLE ingredients (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (length(name) BETWEEN 1 AND 40),
    unit          TEXT NOT NULL CHECK (unit IN ('gram','ml','pcs')),
    stock         INTEGER NOT NULL DEFAULT 0,
    unit_cost_x1000 INTEGER NOT NULL DEFAULT 0 CHECK (unit_cost_x1000 >= 0),
    low_at        INTEGER NOT NULL DEFAULT 0 CHECK (low_at >= 0),
    active        INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);

CREATE TABLE recipe_items (
    menu_id       TEXT NOT NULL REFERENCES menu_items(id),
    ingredient_id TEXT NOT NULL REFERENCES ingredients(id),
    qty           INTEGER NOT NULL CHECK (qty > 0),
    PRIMARY KEY (menu_id, ingredient_id)
);

CREATE TABLE stock_movements (
    id            TEXT PRIMARY KEY,
    ingredient_id TEXT NOT NULL REFERENCES ingredients(id),
    kind          TEXT NOT NULL CHECK (kind IN ('sale','purchase','opname','waste')),
    qty           INTEGER NOT NULL,                 -- signed change
    unit_cost_x1000 INTEGER NOT NULL DEFAULT 0,
    ref           TEXT,
    note          TEXT,
    user_id       TEXT,
    created_at    INTEGER NOT NULL
);
CREATE INDEX idx_stockmov_ing ON stock_movements(ingredient_id, created_at);
CREATE TRIGGER stockmov_no_update BEFORE UPDATE ON stock_movements BEGIN SELECT RAISE(ABORT, 'stock movements are immutable'); END;
CREATE TRIGGER stockmov_no_delete BEFORE DELETE ON stock_movements BEGIN SELECT RAISE(ABORT, 'stock movements are immutable'); END;

-- Read-only API keys. Only the SHA-256 of the key is stored; it is shown once at creation.
CREATE TABLE api_keys (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    token_hash TEXT NOT NULL UNIQUE,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    last_used  INTEGER,
    revoked    INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0,1))
);
