// SPDX-License-Identifier: AGPL-3.0-or-later
// Progressive enhancement only: every page works without this file. No inline script (strict CSP).
const rp = (n) => "Rp" + Math.round(n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ".");

function recalc(form) {
  let sum = 0, count = 0;
  form.querySelectorAll("input.qty").forEach((i) => {
    const q = parseInt(i.value || "0", 10) || 0;
    let unit = parseInt(i.dataset.price || "0", 10);
    const row = i.closest(".menu-item");
    if (row) row.querySelectorAll("input[data-price]:checked").forEach((m) => (unit += parseInt(m.dataset.price || "0", 10)));
    sum += q * unit;
    count += q;
  });
  const out = form.querySelector("[data-cart-total]");
  if (out) out.textContent = count ? `${count} item · ${rp(sum)}` : "Belum ada item";
  const btn = form.querySelector("[data-cart-submit]");
  if (btn) btn.disabled = count === 0 || "locked" in btn.dataset;
}

document.addEventListener("click", (e) => {
  const t = e.target instanceof Element ? e.target : null;
  if (!t) return;
  if (t.closest("[data-print]")) window.print();
  const step = t.closest("[data-step]");
  if (step) {
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
  const form = e.target instanceof Element ? e.target.closest("form[data-cart]") : null;
  if (form) recalc(form);
  const t = e.target;
  if (t instanceof HTMLInputElement && t.dataset.change) {
    const total = parseInt(t.dataset.change, 10);
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
