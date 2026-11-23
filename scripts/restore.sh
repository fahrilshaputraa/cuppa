#!/usr/bin/env bash
# Restore a Cuppa backup. Usage: scripts/restore.sh /path/to/cuppa-YYYY-MM-DD.db
# Stops the service, verifies the backup, keeps the current DB as data/cuppa.db.before-restore, then restores.
set -euo pipefail
BACKUP="${1:?usage: restore.sh <backup.db>}"
DATA="${CUPPA_DATA_DIR:-/opt/cuppa/data}"
BIN="${CUPPA_BIN:-/opt/cuppa/cuppa}"

"$BIN" verify-backup "$BACKUP"                       # refuses corrupt files before touching anything
sudo systemctl stop cuppa || true
[ -f "$DATA/cuppa.db" ] && cp "$DATA/cuppa.db" "$DATA/cuppa.db.before-restore"
rm -f "$DATA/cuppa.db-wal" "$DATA/cuppa.db-shm"
cp "$BACKUP" "$DATA/cuppa.db"
sudo systemctl start cuppa
echo "Restored. Previous database kept at $DATA/cuppa.db.before-restore"
