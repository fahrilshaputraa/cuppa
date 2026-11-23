-- Safe customisation (FR-50): sanitised CSS scoped to the customer page, and one sanitised HTML banner block.
ALTER TABLE outlet_settings ADD COLUMN custom_css TEXT NOT NULL DEFAULT '';
ALTER TABLE outlet_settings ADD COLUMN custom_banner_html TEXT NOT NULL DEFAULT '';
