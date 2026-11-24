// SPDX-License-Identifier: AGPL-3.0-or-later
// Progressive enhancement only: every page works without this file. No inline script (strict CSP).
const EN = document.documentElement.lang === "en";
const rp = (n) => "Rp" + Math.round(n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ".");

function recalc(form) {
  let sum = 0, count = 0;
  form.querySelectorAll("input.qty").forEach((i) => {
    const q = parseInt(i.value || "0", 10) || 0;
    let unit = parseInt(i.dataset.price || "0", 10);
    const row = i.closest(".dish");
    if (row) row.querySelectorAll("input[data-price]:checked").forEach((m) => (unit += parseInt(m.dataset.price || "0", 10)));
    sum += q * unit;
    count += q;
  });
  form.querySelectorAll(".dish").forEach((d) => {
    const q = d.querySelector("input.qty");
    d.classList.toggle("has-qty", !!q && (parseInt(q.value || "0", 10) || 0) > 0);
  });
  const lines = form.querySelector("[data-lines]");
  if (lines) {
    lines.replaceChildren();
    form.querySelectorAll("input.qty").forEach((i) => {
      const q = parseInt(i.value || "0", 10) || 0;
      if (!q) return;
      const d = i.closest(".dish");
      let unit = parseInt(i.dataset.price || "0", 10);
      if (d) d.querySelectorAll("input[data-price]:checked").forEach((m) => (unit += parseInt(m.dataset.price || "0", 10)));
      const li = document.createElement("li");
      const a = document.createElement("span");
      a.textContent = `${q}× ${d ? d.dataset.name : ""}`;
      const b = document.createElement("span");
      b.textContent = rp(q * unit);
      li.append(a, b);
      lines.append(li);
    });
  }
  const out = form.querySelector("[data-cart-total]");
  if (out) out.textContent = count ? `${count} item${EN && count > 1 ? "s" : ""} · ${rp(sum)}` : (EN ? "No items yet" : "Belum ada item");
  const btn = form.querySelector("[data-cart-submit]");
  if (btn) btn.disabled = count === 0 || "locked" in btn.dataset;
}

function applyFilter(root) {
  const q = (root.querySelector("[data-q]")?.value || "").trim().toLowerCase();
  const active = root.querySelector('button[data-cat][aria-pressed="true"]');
  const cat = active ? active.dataset.cat : "";
  let shown = 0;
  root.querySelectorAll("[data-name]").forEach((el) => {
    const ok = (!cat || el.dataset.cat === cat) && (!q || el.dataset.name.toLowerCase().includes(q));
    el.hidden = !ok;
    if (ok) shown++;
  });
  root.querySelectorAll("[data-section]").forEach((s) => (s.hidden = !s.querySelector("[data-name]:not([hidden])")));
  const empty = root.querySelector("[data-empty]");
  if (empty) empty.hidden = shown > 0;
}

document.addEventListener("click", (e) => {
  const t = e.target instanceof Element ? e.target : null;
  if (!t) return;
  if (t.closest("[data-print]")) window.print();
  const open = t.closest("[data-open]");
  if (open) { const dlg = open.closest(".dish")?.querySelector("dialog"); if (dlg && !dlg.open) dlg.showModal(); return; }
  if (t.closest("[data-close]")) { t.closest("dialog")?.close(); return; }
  if (t.tagName === "DIALOG") { t.close(); return; }
  const add = t.closest("[data-add]");
  if (add) {
    const dish = add.closest(".dish");
    const q = dish.querySelector("input.qty");
    if (!(parseInt(q.value || "0", 10) > 0)) q.value = "1";
    q.dispatchEvent(new Event("input", { bubbles: true }));
    add.closest("dialog").close();
    return;
  }
  const cat = t.closest("button[data-cat]");
  if (cat) {
    const root = cat.closest("[data-filter]") || document;
    root.querySelectorAll("button[data-cat]").forEach((b) => b.setAttribute("aria-pressed", String(b === cat)));
    applyFilter(root);
    return;
  }
  const step = t.closest("[data-step]");
  if (step) {
    const dish = step.closest(".dish");
    const q0 = dish && dish.querySelector("input.qty");
    if (dish && dish.hasAttribute("data-opts") && step.dataset.step === "1" && q0 && !(parseInt(q0.value || "0", 10) > 0)) {
      const dlg = dish.querySelector("dialog");
      if (dlg && !dlg.open) { dlg.showModal(); return; }
    }
    const input = step.parentElement.querySelector("input.qty");
    const max = parseInt(input.max || "99", 10);
    input.value = Math.min(max, Math.max(0, (parseInt(input.value || "0", 10) || 0) + parseInt(step.dataset.step, 10)));
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }
  const chip = t.closest("[data-fill]");
  if (chip) {
    const target = document.getElementById(chip.dataset.target);
    if (target) { target.value = chip.dataset.fill; target.dispatchEvent(new Event("input", { bubbles: true })); }
  }
});

document.addEventListener("change", (e) => {
  const form = e.target instanceof Element ? e.target.closest("form[data-cart]") : null;
  if (form) recalc(form);
});

document.addEventListener("input", (e) => {
  const tgt = e.target;
  if (tgt instanceof HTMLInputElement && tgt.hasAttribute("data-q")) applyFilter(tgt.closest("[data-filter]") || document);
  if (tgt instanceof HTMLInputElement && (tgt.hasAttribute("data-margin-price") || tgt.hasAttribute("data-margin-cost"))) {
    const p = parseInt(document.querySelector("[data-margin-price]")?.value || "0", 10) || 0;
    const c = parseInt(document.querySelector("[data-margin-cost]")?.value || "0", 10) || 0;
    const m = document.querySelector("[data-margin]");
    if (m) { m.hidden = !(p > 0 && c > 0); m.textContent = `Margin ${rp(p - c)} per porsi (${Math.round(((p - c) / p) * 100)}%)`; }
  }
  if (e.target && e.target.id === "amt") {
    const t = document.getElementById("t");
    if (t) t.dispatchEvent(new Event("input", { bubbles: true }));
  }
  const form = e.target instanceof Element ? e.target.closest("form[data-cart]") : null;
  if (form) recalc(form);
  const t = e.target;
  if (t instanceof HTMLInputElement && t.dataset.change) {
    const amt = document.getElementById("amt");
    const total = amt && amt.value ? parseInt(amt.value, 10) : parseInt(t.dataset.change, 10);
    const out = document.getElementById("change-out");
    if (out) {
      const diff = (parseInt(t.value || "0", 10) || 0) - total;
      out.textContent = diff >= 0 ? `Kembalian ${rp(diff)}` : `Kurang ${rp(-diff)}`;
      out.className = "change " + (diff >= 0 ? "ok" : "bad");
    }
  }
});

document.querySelectorAll("form[data-cart]").forEach(recalc);
document.querySelectorAll("input[data-change]").forEach((i) => i.dispatchEvent(new Event("input", { bubbles: true })));

document.addEventListener("change", (e) => {
  const sel = e.target;
  if (!(sel instanceof HTMLSelectElement) || !("method" in sel.dataset)) return;
  const cash = sel.value === "cash";
  document.querySelectorAll('[data-for="cash"]').forEach((el) => (el.hidden = !cash));
  document.querySelectorAll('[data-for="noncash"]').forEach((el) => {
    el.hidden = cash;
    const input = el.querySelector("input");
    if (input) input.required = !cash;
  });
  const t = document.getElementById("t");
  if (t) t.required = cash;
});

// Idle screen lock (FR-33): after N seconds without touch/keys on a staff page, drop the session.
(() => {
  const secs = parseInt(document.body.dataset.idle || "0", 10);
  if (!secs) return;
  let t;
  const arm = () => { clearTimeout(t); t = setTimeout(() => { location.href = "/lock"; }, secs * 1000); };
  ["pointerdown", "keydown", "touchstart"].forEach((ev) => document.addEventListener(ev, arm, { passive: true }));
  arm();
})();

// New-order sound (FR-08): beep when the pending count in the cashier queue goes up.
let lastPending = null;
function beep() {
  try {
    const Ctx = window.AudioContext || window.webkitAudioContext;
    const ctx = new Ctx();
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.type = "sine"; o.frequency.value = 880; g.gain.value = 0.15;
    o.connect(g); g.connect(ctx.destination); o.start();
    setTimeout(() => { o.stop(); ctx.close(); }, 350);
  } catch (_) { /* audio needs a user gesture first; ignore */ }
}
document.body.addEventListener("htmx:afterSwap", (e) => {
  const marker = e.target && e.target.querySelector ? e.target.querySelector("[data-pending]") : null;
  if (!marker) return;
  const n = parseInt(marker.dataset.pending || "0", 10);
  if (lastPending !== null && n > lastPending) beep();
  lastPending = n;
});

// PWA: register the service worker (installable on Android/PC; caches public assets only).
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(() => { /* not available over plain http on non-localhost */ });
}

document.addEventListener("change", (e) => {
  const f = e.target;
  if (!(f instanceof HTMLInputElement) || f.type !== "file" || !f.dataset.preview) return;
  const img = document.getElementById(f.dataset.preview);
  const file = f.files && f.files[0];
  if (!img || !file) return;
  img.src = URL.createObjectURL(file);
  img.hidden = false;
  f.closest(".drop")?.classList.add("has");
});
document.querySelectorAll("[data-margin-price]").forEach((i) => i.dispatchEvent(new Event("input", { bubbles: true })));
