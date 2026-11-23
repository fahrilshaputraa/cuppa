document.addEventListener("click", (e) => {
  if (e.target instanceof Element && e.target.closest("[data-print]")) window.print();
});
