// SPDX-License-Identifier: AGPL-3.0-or-later
// Progressive enhancement only: every page works without this file. No inline script (strict CSP).
const rp = (n) => "Rp" + Math.round(n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ".");

function recalc(form) {
  let sum = 0, count = 0;
  form.querySelectorAll("input.qty").forEach((i) => {
    const q = parseInt(i.value || "0", 10) || 0;
    sum += q * parseInt(i.dataset.price || "0", 10);
    count += q;
  });
  const out = form.querySelector("[data-cart-total]");
  if (out) out.textContent = count ? `${count} item · ${rp(sum)}` : "Belum ada item";
  const btn = form.querySelector("[data-cart-submit]");
  if (btn) btn.disabled = count === 0;
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
