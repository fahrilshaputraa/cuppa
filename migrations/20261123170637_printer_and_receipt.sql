-- Receipt + printer settings (printer is optional; receipts can always be printed from the browser).
ALTER TABLE outlet_settings ADD COLUMN printer_host TEXT;            -- "192.168.1.50:9100", private network only
ALTER TABLE outlet_settings ADD COLUMN paper_cols INTEGER NOT NULL DEFAULT 32 CHECK (paper_cols IN (32, 42, 48));
ALTER TABLE outlet_settings ADD COLUMN receipt_footer TEXT NOT NULL DEFAULT 'Terima kasih!';
ALTER TABLE outlet_settings ADD COLUMN open_drawer_on_cash INTEGER NOT NULL DEFAULT 1 CHECK (open_drawer_on_cash IN (0,1));
