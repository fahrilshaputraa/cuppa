# Installation guide / Panduan pemasangan

Target: a non-technical owner (or the person helping them) installs Cuppa in about 30 minutes.
*Tujuan: pemilik kafe atau orang yang membantunya bisa memasang Cuppa dalam ±30 menit.*

## What you need / Yang dibutuhkan
- A small PC / mini-PC / VPS running Ubuntu 22.04+ (or any Linux with Docker).
- A tablet or PC (Chrome/Edge) for the cashier. Optional: ESC/POS LAN receipt printer (58/80 mm) + cash drawer on the printer.
- For customer QR from phones: a domain with HTTPS (VPS), **or** the café Wi-Fi only (`http://<ip-of-pc>:8080`, see step 5).

## Option A — Docker (easiest)
```bash
git clone https://github.com/fahrilshaputraa/cuppa && cd cuppa
cp .env.example .env          # set CUPPA_PUBLIC_URL (https://... in production)
docker compose up -d --build
docker compose logs cuppa | grep "setup code"   # copy the one-time setup code
```
Open `http://localhost:8080/setup`, enter the code, create the owner. The wizard asks for the flow (QR on/off, kitchen on/off) and the local tax rate.

## Option B — single binary + systemd
```bash
cargo build --release                       # or download a release binary
sudo useradd --system --home /opt/cuppa cuppa
sudo mkdir -p /opt/cuppa/data && sudo cp target/release/cuppa /opt/cuppa/ && sudo cp -r static /opt/cuppa/
sudo cp .env.example /opt/cuppa/.env && sudo chown -R cuppa /opt/cuppa
sudo cp deploy/cuppa.service /etc/systemd/system/ && sudo systemctl enable --now cuppa
sudo journalctl -u cuppa | grep "setup code"
```

## HTTPS with nginx + Let's Encrypt (VPS)
```bash
sudo apt install nginx certbot python3-certbot-nginx
sudo cp deploy/nginx.conf /etc/nginx/sites-available/cuppa   # edit the domain
sudo ln -s /etc/nginx/sites-available/cuppa /etc/nginx/sites-enabled/
sudo certbot --nginx -d kafe.contoh.id && sudo nginx -t && sudo systemctl reload nginx
```
Then set in `.env`: `CUPPA_PUBLIC_URL=https://kafe.contoh.id`, `CUPPA_TRUST_PROXY=true` and restart.
Server at the café without a public address? Use a Cloudflare Tunnel in front of nginx.

## First-day checklist / Daftar periksa hari pertama
1. **Settings → Pajak**: enter the rate from your local regulation (PBJT/PB1). Choose inclusive/exclusive.
2. **Menu**: add items (or *Impor CSV*), add variants (Size, Topping) via the *Varian* button.
3. **Meja & QR**: add tables, print each QR and stick it on the table.
4. **Staf**: add cashiers (PIN 4–8 digits). Then, on each cashier tablet, log in as owner → **Perangkat → Daftarkan perangkat ini** → log out. Cashier PINs now work only on registered devices.
5. **Printer**: Settings → printer address (`192.168.x.x:9100`) → **Tes cetak** (also fires the cash drawer).
6. **Shift**: cashier opens the shift with the starting cash.

## Online QRIS (optional)
1. Owner → **Integrasi** → choose Midtrans, paste the **server key** (stored encrypted; `data/secret.key` holds the encryption key, back it up with `data/`).
2. In the Midtrans dashboard set *Payment Notification URL* to `https://<your-domain>/webhooks/midtrans` (needs the public HTTPS setup above).
3. On an order, press **Buat QRIS**; the customer scans the QR on the cashier screen. If a notification is missed, press **Cek status**.
4. Use the sandbox first: set `CUPPA_MIDTRANS_BASE=https://api.sandbox.midtrans.com`.

## Install on the tablet
Open the café URL in Chrome/Edge → menu → **Install app**. Registered device + PIN login works the same as in the browser.

## Updates & backups
- Update: `git pull && docker compose up -d --build` (or replace the binary and restart). If a new version changes the database, Cuppa first writes `data/backups/pre-migrate-<time>.db`.
- Backups: automatic daily to `data/backups/` (last 14). **Also download one weekly** from Settings and keep it off the server.
- **Monthly restore test:** `./cuppa verify-backup data/backups/cuppa-YYYY-MM-DD.db` (checks integrity and shows row counts).
- Restore: `scripts/restore.sh data/backups/cuppa-YYYY-MM-DD.db`.

Quick printer check without logging in: `./cuppa print-test 192.168.1.50:9100 --drawer`.

## Troubleshooting
| Symptom | Fix |
| --- | --- |
| "Perangkat ini belum terdaftar" | Owner logs in on that device and registers it (Perangkat). |
| Printer test fails | Check LAN cable, printer IP, port 9100; printer must be on the same network. |
| QR opens "not found" | Token rotated or table inactive; reprint the QR. Check `CUPPA_PUBLIC_URL`. |
| Cashier locked out | 5 wrong PINs lock the account 5 minutes. |
| No new-order sound | Browsers need one tap on the page first; keep the cashier tab open. |
