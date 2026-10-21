import { connectionIcon } from "../../shared/connection-icons.js";
import { listQuickLaunchItems, renameQuickLaunchItem, removeQuickLaunchItem, reorderQuickLaunchItems } from "./quick-launcher-api.js";
import { moveLauncherItem, visibleLauncherCount } from "./quick-launcher-model.js";
import { openUrlConnection } from "../agent/agent-api.js";

const escape = (value) => String(value ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const dragType = "application/x-gatetree-ops-quick-launch";

export function mountQuickLauncher(host, { onNavigate, showMessage }) {
  let items = [];
  let visibleCount = 0;
  let disposed = false;
  let request = 0;
  let menu = null;
  let dialog = null;
  let animation = null;
  let draggedId = null;
  const busy = new Set();
  const listeners = new AbortController();
  const closeMenu = () => { menu?.remove(); menu = null; };
  const report = (cause) => { if (!disposed) showMessage(String(cause), true); };
  const ids = () => items.map((item) => item.connection.id);

  async function reload() {
    const ticket = ++request;
    try {
      const result = await listQuickLaunchItems();
      if (disposed || ticket !== request) return;
      items = result;
      render();
    } catch (cause) { report(cause); }
  }

  async function launch(id) {
    const item = items.find((item) => item.connection.id === id);
    if (!item || busy.has(id)) return;
    closeMenu();
    busy.add(id);
    try {
      if (item.connection.connectionType === "url") await openUrlConnection(id);
      else onNavigate("connections", { connectionId: id, launchConnection: true });
    } catch (cause) { report(cause); }
    finally { busy.delete(id); }
  }

  function layout() {
    if (disposed) return;
    const buttons = [...host.querySelectorAll(".quick-launch-item")];
    buttons.forEach((button) => { button.hidden = false; });
    const widths = buttons.map((button) => button.getBoundingClientRect().width);
    visibleCount = visibleLauncherCount(widths, host.clientWidth);
    buttons.forEach((button, index) => { button.hidden = index >= visibleCount; });
    const more = host.querySelector(".quick-launch-more");
    if (more) {
      more.hidden = visibleCount === items.length;
      more.title = `${items.length - visibleCount} more connections`;
    }
    closeMenu();
  }

  function render() {
    closeMenu();
    host.innerHTML = items.length ? `<div class="quick-launch-items">${items.map(({ connection: c, label }) => `
      <button class="quick-launch-item is-${escape(c.connectionType)}" draggable="true" data-quick-id="${escape(c.id)}"
        title="${escape(`${c.name}\n${c.target}${c.port ? `:${c.port}` : ""}`)}">${connectionIcon(c.connectionType, c.icon)}<span>${escape(label || c.name)}</span></button>`).join("")}</div>
      <button class="quick-launch-more" aria-label="More quick launch connections" aria-haspopup="menu" title="More connections" hidden>»</button>`
      : `<button class="quick-launch-empty" title="Right-click a connection to pin it, or drag one here">${connectionIcon("ssh")}<span>Quick Launcher · drag connections here</span></button>`;
    host.querySelectorAll("[data-quick-id]").forEach((button) => {
      button.addEventListener("click", () => launch(button.dataset.quickId));
      button.addEventListener("contextmenu", (event) => { event.preventDefault(); context(button.dataset.quickId, event.clientX, event.clientY); });
      button.addEventListener("keydown", (event) => {
        if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
          event.preventDefault(); const rect = button.getBoundingClientRect(); context(button.dataset.quickId, rect.left, rect.bottom);
        }
      });
      button.addEventListener("dragstart", (event) => {
        draggedId = button.dataset.quickId;
        event.dataTransfer.setData(dragType, draggedId);
        event.dataTransfer.effectAllowed = "move";
        closeMenu();
      });
      button.addEventListener("dragend", () => { draggedId = null; clearDrop(); });
    });
    host.querySelector(".quick-launch-more")?.addEventListener("click", overflow);
    host.querySelector(".quick-launch-empty")?.addEventListener("click", () => onNavigate("connections"));
    cancelAnimationFrame(animation);
    animation = requestAnimationFrame(layout);
  }

  function positionMenu(element, x, y) {
    closeMenu();
    menu = element;
    document.body.append(menu);
    menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - menu.offsetWidth - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - menu.offsetHeight - 8))}px`;
    menu.addEventListener("keydown", (event) => {
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const buttons = [...element.querySelectorAll("button:not(:disabled)")];
      const index = buttons.indexOf(document.activeElement);
      const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
        : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
      buttons[next]?.focus();
    });
    menu.querySelector("button")?.focus();
  }

  async function change(operation) {
    closeMenu();
    try { await operation(); }
    catch (cause) { report(cause); reload(); }
  }

  function context(id, x, y) {
    const index = ids().indexOf(id);
    if (index < 0) return;
    const element = document.createElement("div");
    element.className = "quick-launch-menu";
    element.setAttribute("role", "menu");
    element.innerHTML = `<button data-quick-action="open" role="menuitem">Open</button>
      <button data-quick-action="rename" role="menuitem">Rename launcher label…</button>
      <button data-quick-action="left" role="menuitem" ${index === 0 ? "disabled" : ""}>Move left</button>
      <button data-quick-action="right" role="menuitem" ${index === items.length - 1 ? "disabled" : ""}>Move right</button>
      <button data-quick-action="remove" role="menuitem">Remove from Quick Launcher</button>`;
    element.addEventListener("click", (event) => {
      const action = event.target.closest("[data-quick-action]")?.dataset.quickAction;
      if (action === "open") launch(id);
      else if (action === "rename") rename(id);
      else if (action === "remove") change(() => removeQuickLaunchItem(id));
      else if (action === "left" || action === "right") {
        const target = ids()[index + (action === "left" ? -1 : 1)];
        change(() => reorderQuickLaunchItems(moveLauncherItem(ids(), id, target, action === "right")));
      }
    });
    positionMenu(element, x, y);
  }

  function overflow() {
    if (menu) { closeMenu(); return; }
    const element = document.createElement("div");
    element.className = "quick-launch-menu quick-launch-overflow";
    element.setAttribute("role", "menu");
    element.innerHTML = items.slice(visibleCount).map(({ connection: c, label }) => `<div class="quick-launch-overflow-row">
      <button data-quick-open="${escape(c.id)}" role="menuitem" title="${escape(c.target)}">${connectionIcon(c.connectionType, c.icon)}<span>${escape(label || c.name)}</span></button>
      <button data-quick-manage="${escape(c.id)}" aria-label="Manage ${escape(label || c.name)}">⋯</button></div>`).join("");
    element.addEventListener("click", (event) => {
      const open = event.target.closest("[data-quick-open]");
      const manage = event.target.closest("[data-quick-manage]");
      if (open) launch(open.dataset.quickOpen);
      if (manage) { const rect = manage.getBoundingClientRect(); context(manage.dataset.quickManage, rect.left, rect.bottom); }
    });
    const rect = host.querySelector(".quick-launch-more").getBoundingClientRect();
    positionMenu(element, rect.right - 240, rect.bottom + 6);
  }

  function rename(id) {
    const item = items.find((item) => item.connection.id === id);
    if (!item) return;
    closeMenu();
    dialog?.remove();
    dialog = document.createElement("dialog");
    dialog.className = "quick-launch-rename";
    dialog.innerHTML = `<form method="dialog"><strong>Rename launcher label</strong>
      <label>Label<input maxlength="80" value="${escape(item.label || "")}" placeholder="${escape(item.connection.name)}" /></label>
      <small>Leave empty to use the connection name.</small><footer><button type="button" data-rename-cancel>Cancel</button><button type="submit" value="save">Save</button></footer></form>`;
    const current = dialog;
    current.querySelector("[data-rename-cancel]").addEventListener("click", () => current.close("cancel"));
    current.addEventListener("close", () => {
      if (current.returnValue === "save" && !disposed) change(() => renameQuickLaunchItem(id, current.querySelector("input").value.trim() || null));
      current.remove(); if (dialog === current) dialog = null;
    }, { once: true });
    document.body.append(current);
    current.showModal(); current.querySelector("input").select();
  }

  function clearDrop() {
    host.classList.remove("is-launcher-drop-target");
    host.querySelectorAll(".is-launcher-insert").forEach((button) => button.classList.remove("is-launcher-insert"));
  }
  host.addEventListener("dragover", (event) => {
    if (!draggedId || !Array.from(event.dataTransfer.types).includes(dragType)) return;
    event.preventDefault(); clearDrop(); host.classList.add("is-launcher-drop-target");
    event.target.closest("[data-quick-id]")?.classList.add("is-launcher-insert");
  }, { signal: listeners.signal });
  host.addEventListener("dragleave", (event) => { if (!host.contains(event.relatedTarget)) clearDrop(); }, { signal: listeners.signal });
  host.addEventListener("drop", (event) => {
    if (!draggedId || event.dataTransfer.getData(dragType) !== draggedId) return;
    event.preventDefault();
    const target = event.target.closest("[data-quick-id]");
    const after = target ? event.clientX > target.getBoundingClientRect().left + target.getBoundingClientRect().width / 2 : false;
    const next = moveLauncherItem(ids(), draggedId, target?.dataset.quickId, after);
    clearDrop(); draggedId = null;
    change(() => reorderQuickLaunchItems(next));
  }, { signal: listeners.signal });
  window.addEventListener("gatetree-ops:quick-launch-changed", reload, { signal: listeners.signal });
  window.addEventListener("gatetree-ops:connections-changed", reload, { signal: listeners.signal });
  document.addEventListener("pointerdown", (event) => { if (menu && !menu.contains(event.target) && !event.target.closest(".quick-launch-more")) closeMenu(); }, { signal: listeners.signal });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") closeMenu(); }, { signal: listeners.signal });
  window.addEventListener("blur", closeMenu, { signal: listeners.signal });
  window.addEventListener("resize", layout, { signal: listeners.signal });
  const observer = new ResizeObserver(layout);
  observer.observe(host);
  reload();
  return () => {
    disposed = true; request++; listeners.abort(); observer.disconnect();
    cancelAnimationFrame(animation); closeMenu(); dialog?.remove(); clearDrop();
  };
}
