-- Short description shown on the customer menu and the item detail sheet.
ALTER TABLE menu_items ADD COLUMN description TEXT CHECK (description IS NULL OR length(description) <= 240);
