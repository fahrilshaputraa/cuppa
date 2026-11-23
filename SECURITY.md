# Security Policy

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting:
<https://github.com/fahrilshaputraa/cuppa/security/advisories/new>.
You will get an acknowledgement within 72 hours.

## Supported versions

Only the latest release on `main` receives fixes while the project is pre-1.0.

## Security model

- **Trust boundary:** staff are authenticated; customers are anonymous and can only read the menu and create `pending` orders for an active table token.
- **Table tokens are not secrets.** Safeguards: cashier confirmation before the kitchen, per-IP and per-token rate limits, a cap of pending orders per table, auto-cancel of unconfirmed orders, token rotation.
- **Never trusted from the client:** prices, totals, roles, order status.
- **Money integrity** is enforced by the database (triggers), not only by application code.
- **No code execution of user content.** Custom CSS/JS (PRD FR-50/56) is out of scope until a dedicated review.
- **Card data:** only an approval/reference number is stored; card-number-like input is rejected.

## Deploying safely

1. Serve over HTTPS and set `CUPPA_PUBLIC_URL=https://…` (enables `Secure` cookies and HSTS).
2. Keep `CUPPA_BIND` on loopback; expose only the reverse proxy.
3. Set `CUPPA_TRUST_PROXY=true` only behind your own proxy.
4. Back up `data/` regularly and test restores.
5. Use a strong owner passphrase; cashiers use 4–8 digit PINs on registered café devices only.
