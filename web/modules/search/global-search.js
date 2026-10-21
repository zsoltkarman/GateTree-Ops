import { connectionIcon } from "../../shared/connection-icons.js";
import { escapeHtml } from "../../shared/ui.js";
import { SEARCH_CATEGORIES, searchEntries } from "./search-index.js";
import { loadSearchIndex } from "./search-api.js";
import { listQuickLaunchItems, pinQuickLaunchItem } from "../quick-launcher/quick-launcher-api.js";

export function mountGlobalSearch(shell, { onNavigate }) {
  const input = shell.querySelector(".global-search input");
  const host = shell.querySelector(".global-search");
  const panel = document.createElement("section");
  panel.className = "global-search-panel";
  panel.hidden = true;
  panel.setAttribute("aria-label", "Workspace search");
  panel.innerHTML = '<div class="search-filters" role="group" aria-label="Filter results"></div><p class="search-status" role="status" aria-live="polite"></p><div class="search-body"><div class="search-table"><div class="search-table-heading" aria-hidden="true"><span>Type</span><span>Name</span><span>Folder</span></div><div class="search-results" id="workspace-search-results" role="listbox" aria-label="Search results"></div></div><aside class="search-preview" tabindex="0" aria-label="Selected result preview"></aside></div><footer class="search-footer"><span>↑ ↓ Select · Enter Open · Esc Close</span><button type="button" data-search-refresh>Refresh</button></footer>';
  host.append(panel);
  const results = panel.querySelector(".search-results");
  const filters = panel.querySelector(".search-filters");
  const status = panel.querySelector(".search-status");
  const preview = panel.querySelector(".search-preview");
  input.placeholder = "Search everything…";
  input.setAttribute("aria-label", "Search everything");
  input.setAttribute("role", "combobox");
  input.setAttribute("aria-autocomplete", "list");
  input.setAttribute("aria-controls", results.id);
  input.setAttribute("aria-expanded", "false");
  const lifetime = new AbortController();
  let loadingController = null;
  let entries = [], unavailable = [], visible = [];
  let loading = false, loaded = false, category = "All", active = -1, timer;
  let quickLaunchIds = new Set(), quickLaunchRequest = 0;
  const pinBusy = new Set(), pinErrors = new Map();

  function close() {
    panel.hidden = true;
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
    active = -1;
  }

  function select(index, scroll = false) {
    active = index;
    results.querySelectorAll('[role="option"]').forEach((option, i) => {
      option.setAttribute("aria-selected", String(i === active));
    });
    const option = results.querySelector(`[data-search-index="${active}"]`);
    if (option) {
      input.setAttribute("aria-activedescendant", option.id);
      if (scroll) option.scrollIntoView({ block: "nearest" });
    } else input.removeAttribute("aria-activedescendant");
    renderPreview();
  }

  function renderPreview() {
    const entry = visible[active];
    if (!entry) {
      preview.innerHTML = '<p class="search-preview-label">Selected result</p><p class="search-preview-empty">Select a result to see its details here.</p>';
      return;
    }
    const query = input.value.trim();
    const connectionId = entry.category === "Connections" ? entry.detail?.connectionId : null;
    const pinned = quickLaunchIds.has(connectionId);
    const focusedPin = document.activeElement?.dataset?.searchPin === connectionId;
    const scrollTop = preview.dataset.entryId === entry.id ? preview.scrollTop : 0;
    if (focusedPin) preview.focus({ preventScroll: true });
    const fields = [
      ["Folder / Path", entry.location], ["Target", entry.target],
      ["Tags", entry.tags], ["Status", entry.status], ["Description", entry.description],
    ].filter(([, value]) => value);
    preview.innerHTML = `<p class="search-preview-label">Selected result</p><h3>${highlight(entry.title, query)}</h3><span class="search-preview-type">${entry.connectionType ? connectionIcon(entry.connectionType, entry.icon) : ""}${escapeHtml(entry.type)}</span><dl>${fields.map(([label, value]) => `<div><dt>${label}</dt><dd>${highlight(value, query)}</dd></div>`).join("")}</dl>${entry.matchedBody && entry.excerpt ? `<div class="search-preview-excerpt"><strong>Matching text</strong><p>${highlight(entry.excerpt, query)}</p></div>` : ""}<button type="button" class="search-preview-open" data-search-open>Open <span aria-hidden="true">↵</span></button>`;
    if (connectionId) {
      preview.insertAdjacentHTML("beforeend", `<button type="button" class="search-preview-pin" data-search-pin="${escapeHtml(connectionId)}" ${pinned || pinBusy.has(connectionId) ? "disabled" : ""}>${pinned ? "Added to Quick Launcher ✓" : pinBusy.has(connectionId) ? "Adding…" : "Add to Quick Launcher"}</button>${pinErrors.has(connectionId) ? `<p class="search-preview-pin-error" role="status">${escapeHtml(pinErrors.get(connectionId))}</p>` : ""}`);
    }
    preview.dataset.entryId = entry.id;
    preview.scrollTop = scrollTop;
    if (focusedPin) preview.querySelector('[data-search-pin]:not(:disabled)')?.focus({ preventScroll: true });
  }

  async function refreshQuickLaunch() {
    const request = ++quickLaunchRequest;
    try {
      const items = await listQuickLaunchItems();
      if (lifetime.signal.aborted || request !== quickLaunchRequest) return;
      quickLaunchIds = new Set(items.map(item => item.connection.id));
      if (!panel.hidden) renderPreview();
    } catch {
      // Search remains usable; pinning is idempotent even if this read fails.
    }
  }

  async function pinResult(id) {
    if (!visible.some(entry => entry.category === "Connections" && entry.detail?.connectionId === id) || pinBusy.has(id) || quickLaunchIds.has(id)) return;
    pinBusy.add(id);
    pinErrors.delete(id);
    renderPreview();
    try {
      const items = await pinQuickLaunchItem(id);
      if (lifetime.signal.aborted) return;
      ++quickLaunchRequest;
      quickLaunchIds = new Set(items.map(item => item.connection.id));
    } catch (error) {
      if (!lifetime.signal.aborted) pinErrors.set(id, String(error));
    } finally {
      pinBusy.delete(id);
      if (!lifetime.signal.aborted && !panel.hidden) renderPreview();
    }
  }

  function openResult(index) {
    const result = visible[index];
    if (!result) return;
    close();
    onNavigate(result.module, result.detail);
    input.blur();
  }

  function render() {
    if (lifetime.signal.aborted || panel.hidden) return;
    const query = input.value.trim();
    const matches = searchEntries(entries, query);
    const counts = new Map(SEARCH_CATEGORIES.map((name) => [name, matches.filter((entry) => entry.category === name).length]));
    filters.innerHTML = ["All", ...SEARCH_CATEGORIES.filter((name) => counts.get(name) || category === name)].map((name) =>
      `<button type="button" data-search-category="${name}" aria-pressed="${category === name}">${name}${query ? ` <small>${name === "All" ? matches.length : counts.get(name)}</small>` : ""}</button>`).join("");
    const filtered = category === "All" ? matches : matches.filter((entry) => entry.category === category);
    const previousId = visible[active]?.id;
    visible = filtered.slice(0, 60);
    results.innerHTML = visible.map((entry, index) => {
      const folder = entry.location || "—";
      return `<div class="search-result" role="option" aria-selected="false" id="workspace-search-result-${index}" data-search-index="${index}"><span class="search-result-type">${entry.connectionType ? connectionIcon(entry.connectionType, entry.icon) : ""}${escapeHtml(entry.type)}</span><strong class="search-result-name">${highlight(entry.title, query)}</strong><span class="search-result-location">${highlight(folder, query)}</span></div>`;
    }).join("");
    results.querySelectorAll('[role="option"]').forEach((option, index) => {
      option.title = `${visible[index].title}\n${visible[index].location || ""}\nDouble-click to open`;
    });
    if (filtered.length > visible.length) results.insertAdjacentHTML("beforeend", '<p class="search-status">Showing the first 60 results. Narrow your search to find more.</p>');
    status.textContent = !query ? "Search names, tags, hosts, skill contents and note text." : `${filtered.length} result${filtered.length === 1 ? "" : "s"}${loading ? " · Searching…" : ""}`;
    if (!query && loading) status.textContent += " Loading workspace…";
    if (unavailable.length) status.textContent += ` · Unavailable: ${unavailable.join(", ")}. Refresh to retry.`;
    if (query && !filtered.length) results.innerHTML = `<p class="search-empty">${loading ? "Searching workspace…" : "No matches. Try a name, tag, hostname or a different word."}</p>`;
    const previous = visible.findIndex((entry) => entry.id === previousId);
    select(previous >= 0 ? previous : visible.length ? 0 : -1);
  }

  async function refresh() {
    loadingController?.abort();
    const controller = loadingController = new AbortController();
    loading = true;
    loaded = true;
    render();
    await loadSearchIndex({ signal: controller.signal, onProgress: (snapshot) => {
      if (controller.signal.aborted || lifetime.signal.aborted) return;
      ({ entries, unavailable, loading } = snapshot);
      render();
    } });
  }

  function open() {
    const wasClosed = panel.hidden;
    panel.hidden = false;
    input.setAttribute("aria-expanded", "true");
    render();
    // Refresh on each new search session, including changes made in other modules.
    if (wasClosed || !loaded) { void refresh(); void refreshQuickLaunch(); }
  }

  input.addEventListener("focus", open, { signal: lifetime.signal });
  input.addEventListener("click", () => { if (panel.hidden) open(); }, { signal: lifetime.signal });
  input.addEventListener("input", () => {
    clearTimeout(timer);
    active = -1;
    if (panel.hidden) open();
    timer = setTimeout(render, 100);
  }, { signal: lifetime.signal });
  input.addEventListener("keydown", (event) => {
    if (event.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); close(); return; }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (panel.hidden) open();
      if (visible.length) select((active + (event.key === "ArrowDown" ? 1 : -1) + visible.length) % visible.length, true);
    } else if (event.key === "Enter" && !panel.hidden) {
      event.preventDefault();
      clearTimeout(timer);
      render();
      openResult(active);
    }
  }, { signal: lifetime.signal });
  panel.addEventListener("mousedown", (event) => {
    if (!preview.contains(event.target)) event.preventDefault();
  }, { signal: lifetime.signal });
  panel.addEventListener("keydown", (event) => {
    if (event.key === "Escape") { event.preventDefault(); input.focus(); close(); }
    else if (event.key === "Enter" && preview.contains(event.target) && !event.target.closest("button")) {
      event.preventDefault(); openResult(active);
    }
  }, { signal: lifetime.signal });
  panel.addEventListener("click", (event) => {
    const filter = event.target.closest("[data-search-category]");
    if (filter) {
      category = filter.dataset.searchCategory;
      active = -1;
      results.scrollTop = 0;
      render();
      input.focus();
    }
    const result = event.target.closest("[data-search-index]");
    if (result) select(Number(result.dataset.searchIndex));
    if (event.target.closest("[data-search-open]")) openResult(active);
    const pin = event.target.closest("[data-search-pin]");
    if (pin && !pin.disabled) void pinResult(pin.dataset.searchPin);
    if (event.target.closest("[data-search-refresh]")) void refresh();
  }, { signal: lifetime.signal });
  results.addEventListener("dblclick", (event) => {
    const result = event.target.closest("[data-search-index]");
    if (result) openResult(Number(result.dataset.searchIndex));
  }, { signal: lifetime.signal });
  document.addEventListener("pointerdown", (event) => {
    if (!host.contains(event.target)) close();
  }, { signal: lifetime.signal });
  host.addEventListener("focusout", (event) => {
    if (!host.contains(event.relatedTarget)) close();
  }, { signal: lifetime.signal });
  window.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault(); input.focus(); input.select();
      if (panel.hidden) open();
    }
  }, { signal: lifetime.signal });

  window.addEventListener("gatetree-ops:connections-changed", () => {
    loaded = false;
    if (!panel.hidden) void refresh();
    else { loadingController?.abort(); loading = false; }
  }, { signal: lifetime.signal });
  window.addEventListener("gatetree-ops:quick-launch-changed", () => {
    if (!panel.hidden) void refreshQuickLaunch();
  }, { signal: lifetime.signal });

  return () => {
    clearTimeout(timer);
    lifetime.abort();
    loadingController?.abort();
    entries = []; visible = [];
    panel.remove();
  };
}

function highlight(text, query) {
  const terms = query.trim().split(/\s+/u).filter(Boolean).map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (!terms.length) return escapeHtml(text);
  const pattern = new RegExp(terms.sort((a, b) => b.length - a.length).join("|"), "giu");
  let html = "", previous = 0;
  for (const match of String(text).matchAll(pattern)) {
    html += escapeHtml(text.slice(previous, match.index)) + `<mark>${escapeHtml(match[0])}</mark>`;
    previous = match.index + match[0].length;
  }
  return html + escapeHtml(text.slice(previous));
}
