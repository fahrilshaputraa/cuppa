-- Refunds sent back through the payment gateway (FR-42). Immutable, idempotent per refund_key.
CREATE TABLE gateway_refunds (
    id                TEXT PRIMARY KEY,
    provider_order_id TEXT NOT NULL,
    refund_key        TEXT NOT NULL UNIQUE,
    amount            INTEGER NOT NULL CHECK (amount > 0),
    created_at        INTEGER NOT NULL
);
CREATE TRIGGER gwrefund_no_update BEFORE UPDATE ON gateway_refunds BEGIN SELECT RAISE(ABORT, 'gateway refunds are immutable'); END;
CREATE TRIGGER gwrefund_no_delete BEFORE DELETE ON gateway_refunds BEGIN SELECT RAISE(ABORT, 'gateway refunds are immutable'); END;
