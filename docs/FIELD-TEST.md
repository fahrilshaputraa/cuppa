# Field test checklist (PRD step 8)

Goal: four weeks of daily use in one real café without losing data. This list verifies what automated tests cannot:
real printers, the cash drawer, a real QRIS account, real phones and tablets. Tick each item and report failures with the
template *Field report* (GitHub → Issues → New issue).

## 0. Before opening (30 min)
- [ ] Install on a clean machine following `docs/INSTALL.md`; note every step that was unclear and how long it took.
- [ ] Complete the setup wizard. Write down the tax rate you used and where you got it (your regional regulation).
- [ ] Log in as owner on each cashier tablet → **Perangkat → Daftarkan** → log out. Confirm the cashier PIN works there and **not** on a phone.
- [ ] Install the PWA on the tablet (Chrome menu → *Install app*) and reopen it from the home screen.

## 1. Printer and cash drawer (needs the real hardware)
Quick check from the server, no login needed:

```bash
./cuppa print-test 192.168.1.50:9100 --drawer      # receipt printer: prints a test slip and pulses the drawer
```
- [ ] Slip prints, cut works, characters look right (58 mm vs 80 mm: adjust *Lebar kertas* in Settings).
- [ ] Drawer opens (RJ11 cable from drawer to printer; some drawers need 12 V vs 24 V — see the printer manual).
- [ ] In the app: Settings → printer address → **Tes cetak**.
- [ ] Pay a cash order: receipt auto-prints and the drawer opens. Pay by QRIS: drawer stays shut.
- [ ] Unplug the printer, pay an order: the sale must still complete (the receipt can be reprinted later from the order page).
- [ ] Kitchen mode: confirm a QR order → ticket prints on the kitchen/bar printer.
Note the printer model, connection and any quirk in the report.

## 2. Money flow (day 1, with a cashier)
- [ ] Open shift with the real starting cash. Take 5 cash sales, 2 QRIS (manual reference), 1 split payment.
- [ ] Give one discount below the cashier limit and one above it (owner approves with their password).
- [ ] Void one paid order and refund one partially. Check the audit log names who did it and who approved.
- [ ] Close the shift **blind** (count the drawer first). The difference should be Rp0; if not, find out why before continuing.
- [ ] Compare the day's report total with your own paper tally.

## 3. Customers (day 2)
- [ ] Print the table QRs, stick them on tables, scan with 3 different phones (Android + iPhone). Order, wait for confirmation, view status.
- [ ] Ask a customer to order without help — note where they hesitate.
- [ ] Ask for the bill / call a waiter from the table.
- [ ] Switch to English on one phone.
- [ ] Order while the café is "closed" (Settings → hours) to confirm the message.

## 4. QRIS online (optional, needs a Midtrans account)
- [ ] Use the **sandbox** first: `CUPPA_MIDTRANS_BASE=https://api.sandbox.midtrans.com`, sandbox server key, notification URL `https://<domain>/webhooks/midtrans`.
- [ ] Order → **Buat QRIS** → pay with the Midtrans simulator → the order must turn paid within seconds and a receipt prints.
- [ ] Stop the server for a minute, pay, start it again → **Cek status** must reconcile.
- [ ] Only then switch to production keys and test with a small real payment.

## 5. Safety
- [ ] Pull the power cable while idle; boot again. Everything is still there, the service starts by itself.
- [ ] `./cuppa verify-backup data/backups/<today>.db` prints OK. Restore it on another machine with `scripts/restore.sh`.
- [ ] From outside the café network, open `https://<domain>/` and `/owner` (must ask for login) and `/healthz`.
- [ ] Try 5 wrong PINs: the account must lock for 5 minutes.

## What to send back
For every failure: what you did, what you expected, what happened, a photo of the printer slip if relevant, and the last
lines of the server log (`docker compose logs cuppa --tail 100` or `journalctl -u cuppa -n 100`). Never paste real card
numbers, passwords or API keys.
