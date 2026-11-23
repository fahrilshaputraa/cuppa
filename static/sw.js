// Cuppa service worker. Caches only public static assets and an offline notice.
// Authenticated pages (orders, money, staff) are NEVER cached, so a shared tablet leaks nothing.
const VERSION = "cuppa-v1";
const SHELL = [
  "/static/css/app.css", "/static/css/tokens.css", "/static/js/app.js",
  "/static/vendor/htmx.min.js", "/static/vendor/htmx-sse.js",
  "/static/fonts/figtree-latin-wght-normal.woff2",
  "/static/icons/icon-192.png", "/static/manifest.webmanifest", "/offline"
];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(VERSION).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(caches.keys().then((ks) => Promise.all(ks.filter((k) => k !== VERSION).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== self.location.origin) return;
  if (url.pathname.startsWith("/static/")) {
    // static assets: cache first, refresh in the background
    e.respondWith(caches.match(req).then((hit) => {
      const net = fetch(req).then((res) => { if (res.ok) caches.open(VERSION).then((c) => c.put(req, res.clone())); return res; }).catch(() => hit);
      return hit || net;
    }));
    return;
  }
  if (req.mode === "navigate") {
    // pages: network only; if the café server is unreachable show the offline notice
    e.respondWith(fetch(req).catch(() => caches.match("/offline")));
  }
});
