-- Payment gateway (FR-41/42), outgoing webhooks (FR-62), scheduled prices (FR-49).

ALTER TABLE outlet_settings ADD COLUMN gateway_provider TEXT NOT NULL DEFAULT '' CHECK (gateway_provider IN ('','midtrans'));
ALTER TABLE outlet_settings ADD COLUMN gateway_key_enc TEXT;        -- AES-256-GCM, key outside the database
ALTER TABLE outlet_settings ADD COLUMN webhook_url TEXT;
ALTER TABLE outlet_settings ADD COLUMN webhook_secret_enc TEXT;

CREATE TABLE gateway_charges (
    id                TEXT PRIMARY KEY,
    order_id          TEXT NOT NULL REFERENCES orders(id),
    provider          TEXT NOT NULL,
    provider_order_id TEXT NOT NULL UNIQUE,
    amount            INTEGER NOT NULL CHECK (amount > 0),
    status            TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','settled','expired','failed','unapplied')),
    qr_string         TEXT,
    created_by        TEXT NOT NULL REFERENCES users(id),
    created_at        INTEGER NOT NULL,
    settled_at        INTEGER
);
CREATE INDEX idx_charges_order ON gateway_charges(order_id);
CREATE INDEX idx_charges_status ON gateway_charges(status);

-- Time-of-day prices, e.g. happy hour (FR-49). The order line copies the price actually charged.
CREATE TABLE price_rules (
    id       TEXT PRIMARY KEY,
    menu_id  TEXT NOT NULL REFERENCES menu_items(id),
    price    INTEGER NOT NULL CHECK (price >= 0),
    from_hm  TEXT NOT NULL,
    to_hm    TEXT NOT NULL,
    label    TEXT,
    active   INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);
CREATE INDEX idx_price_rules_menu ON price_rules(menu_id);
