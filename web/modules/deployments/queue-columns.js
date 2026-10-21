const STORAGE_KEY = "gatetree-ops.queue-column-widths.v1";
const MINIMUMS = [26, 90, 100, 100, 75, 110];

export function resizeQueueColumns(widths, boundary, delta) {
  const result = [...widths];
  const shift = Math.max(MINIMUMS[boundary] - widths[boundary], Math.min(delta, widths[boundary + 1] - MINIMUMS[boundary + 1]));
  result[boundary] += shift; result[boundary + 1] -= shift;
  return result;
}
function readWidths() {
  try {
    const widths = JSON.parse(localStorage.getItem(STORAGE_KEY));
    return Array.isArray(widths) && widths.length === MINIMUMS.length && widths.every(n => Number.isFinite(n) && n > 0 && n <= 1) ? widths : null;
  } catch (_) { return null; }
}
function template(widths) { return widths.map((width, index) => `minmax(${MINIMUMS[index]}px,${width.toFixed(6)}fr)`).join(" "); }

export function mountQueueColumns(root, { onResizing = () => {} } = {}) {
  const module = root.querySelector(".queue-module");
  if (!module) return () => {};
  let drag = null;
  let weights = readWidths();
  if (weights) module.style.setProperty("--queue-columns", template(weights));
  function measure(handle) { return [...handle.closest(".queue-job-header").children].map(cell => cell.getBoundingClientRect().width); }
  function apply(widths) {
    const total = widths.reduce((sum, width) => sum + width, 0);
    if (!total) return;
    weights = widths.map(width => width / total);
    module.style.setProperty("--queue-columns", template(weights));
  }
  function save() { try { localStorage.setItem(STORAGE_KEY, JSON.stringify(weights)); } catch (_) {} }
  function reset() {
    module.style.removeProperty("--queue-columns"); weights = null;
    try { localStorage.removeItem(STORAGE_KEY); } catch (_) {}
  }
  function finish() {
    if (!drag) return;
    const current = drag; drag = null;
    current.events.abort();
    module.classList.remove("is-resizing-columns");
    try { if (current.handle.hasPointerCapture(current.id)) current.handle.releasePointerCapture(current.id); } catch (_) {}
    save(); onResizing(false);
  }
  module.querySelectorAll("[data-resize-queue-column]").forEach(handle => {
    const boundary = Number(handle.dataset.resizeQueueColumn);
    handle.addEventListener("pointerdown", event => {
      if (event.button !== 0 || drag) return;
      const widths = measure(handle);
      if (widths.length !== MINIMUMS.length || widths.some((n, i) => !Number.isFinite(n) || n < MINIMUMS[i])) return;
      event.preventDefault(); event.stopPropagation();
      const events = new AbortController();
      drag = { handle, id: event.pointerId, events };
      module.classList.add("is-resizing-columns"); onResizing(true);
      try { handle.setPointerCapture(event.pointerId); } catch (_) {}
      const move = event => {
        if (event.pointerId !== drag?.id) return;
        apply(resizeQueueColumns(widths, boundary, event.clientX - startX));
      };
      const startX = event.clientX;
      const end = event => { if (event.pointerId === drag?.id) finish(); };
      window.addEventListener("pointermove", move, { signal: events.signal });
      window.addEventListener("pointerup", end, { signal: events.signal });
      window.addEventListener("pointercancel", end, { signal: events.signal });
      window.addEventListener("blur", finish, { signal: events.signal });
      handle.addEventListener("lostpointercapture", finish, { signal: events.signal });
    });
    handle.addEventListener("keydown", event => {
      if (!["ArrowLeft", "ArrowRight", "Home"].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      if (event.key === "Home") { reset(); return; }
      const widths = measure(handle);
      if (widths.some((n, i) => n < MINIMUMS[i])) return;
      apply(resizeQueueColumns(widths, boundary, (event.key === "ArrowRight" ? 1 : -1) * (event.shiftKey ? 25 : 10))); save();
    });
    handle.addEventListener("dblclick", event => { event.preventDefault(); event.stopPropagation(); reset(); });
  });
  return finish;
}
