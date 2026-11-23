-- PRD core: modifiers, discounts, rounding, refunds, cash movements, permissions, devices, stock, hours, flow modes.

-- Flow modes (FR-27), hours (FR-05/26), rounding, discount limit, theme (FR-30), devices (FR-32), kitchen printer (FR-21)
ALTER TABLE outlet_settings ADD COLUMN kitchen_mode INTEGER NOT NULL DEFAULT 0 CHECK (kitchen_mode IN (0,1));
ALTER TABLE outlet_settings ADD COLUMN wizard_done INTEGER NOT NULL DEFAULT 0 CHECK (wizard_done IN (0,1));
ALTER TABLE outlet_settings ADD COLUMN open_from TEXT;                  -- 'HH:MM' local, NULL = always open
ALTER TABLE outlet_settings ADD COLUMN open_to TEXT;
ALTER TABLE outlet_settings ADD COLUMN round_to INTEGER NOT NULL DEFAULT 0 CHECK (round_to IN (0,100,500,1000));
ALTER TABLE outlet_settings ADD COLUMN cashier_discount_max_bp INTEGER NOT NULL DEFAULT 1000 CHECK (cashier_discount_max_bp BETWEEN 0 AND 10000);
ALTER TABLE outlet_settings ADD COLUMN theme TEXT NOT NULL DEFAULT 'copper' CHECK (theme IN ('copper','green','blue','rose'));
ALTER TABLE outlet_settings ADD COLUMN require_device INTEGER NOT NULL DEFAULT 1 CHECK (require_device IN (0,1));
ALTER TABLE outlet_settings ADD COLUMN kitchen_printer_host TEXT;

-- Stock per menu item (FR-24): NULL = untracked
ALTER TABLE menu_items ADD COLUMN stock INTEGER CHECK (stock IS NULL OR stock >= 0);

-- Orders: discount, rounding (own line), table moves tracked via audit
ALTER TABLE orders ADD COLUMN discount INTEGER NOT NULL DEFAULT 0 CHECK (discount >= 0);
ALTER TABLE orders ADD COLUMN rounding INTEGER NOT NULL DEFAULT 0;
ALTER TABLE order_items ADD COLUMN voided INTEGER NOT NULL DEFAULT 0 CHECK (voided IN (0,1));
ALTER TABLE order_items ADD COLUMN void_reason TEXT;

-- Modifiers (FR-03)
CREATE TABLE modifier_groups (
    id         TEXT PRIMARY KEY,
    menu_id    TEXT NOT NULL REFERENCES menu_items(id),
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    required   INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0,1)),
    max_select INTEGER NOT NULL DEFAULT 1 CHECK (max_select BETWEEN 1 AND 10)
);
CREATE TABLE modifiers (
    id       TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES modifier_groups(id),
    name     TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    price    INTEGER NOT NULL DEFAULT 0 CHECK (price >= 0),
    active   INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);
CREATE INDEX idx_modgroups_menu ON modifier_groups(menu_id);

-- Refunds are separate immutable documents (FR-16)
CREATE TABLE refunds (
    id          TEXT PRIMARY KEY,
    order_id    TEXT NOT NULL REFERENCES orders(id),
    amount      INTEGER NOT NULL CHECK (amount > 0),
    reason      TEXT NOT NULL,
    actor_id    TEXT NOT NULL REFERENCES users(id),
    approver_id TEXT NOT NULL REFERENCES users(id),
    created_at  INTEGER NOT NULL
);
CREATE TRIGGER refunds_no_update BEFORE UPDATE ON refunds BEGIN SELECT RAISE(ABORT, 'refunds are immutable'); END;
CREATE TRIGGER refunds_no_delete BEFORE DELETE ON refunds BEGIN SELECT RAISE(ABORT, 'refunds are immutable'); END;

-- Cash in / out of the drawer without a sale (FR-17)
CREATE TABLE cash_movements (
    id         TEXT PRIMARY KEY,
    shift_id   TEXT NOT NULL REFERENCES shifts(id),
    kind       TEXT NOT NULL CHECK (kind IN ('in','out','open_drawer')),
    amount     INTEGER NOT NULL DEFAULT 0 CHECK (amount >= 0),
    reason     TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);
CREATE TRIGGER cash_no_update BEFORE UPDATE ON cash_movements BEGIN SELECT RAISE(ABORT, 'cash movements are immutable'); END;
CREATE TRIGGER cash_no_delete BEFORE DELETE ON cash_movements BEGIN SELECT RAISE(ABORT, 'cash movements are immutable'); END;

-- Per-user permission overrides on top of role defaults (FR-31)
CREATE TABLE user_permission_overrides (
    user_id TEXT NOT NULL REFERENCES users(id),
    perm    TEXT NOT NULL,
    allowed INTEGER NOT NULL CHECK (allowed IN (0,1)),
    PRIMARY KEY (user_id, perm)
);

-- Registered devices for PIN login (FR-32)
CREATE TABLE devices (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    token_hash TEXT NOT NULL UNIQUE,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    last_seen  INTEGER,
    revoked    INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0,1))
);

-- Extended order status for refunds: keep 'paid' immutable; refunds live in their own table.
