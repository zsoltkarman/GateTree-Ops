import { escapeHtml } from '../../shared/ui.js';
import { connectionIcon } from '../../shared/connection-icons.js';
import { moveTab, nextTabAfterClose } from './tab-model.js';

const symbol = type => ['ssh', 'rdp', 'local'].includes(type) ? connectionIcon(type === 'rdp' ? 'rdp' : 'ssh') : `<svg viewBox="0 0 24 24" aria-hidden="true">${type === 'plan' ? '<path d="M5 5h14v4H5zM5 15h14v4H5zM12 9v6"/>' : '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>'}</svg>`;
export function mountWorkspaceTabs(bar, panels, { onActivate = () => {}, onError = () => {} } = {}) {
  let tabs = [], active = null, dragged = null, disposed = false;
  const lifetime = new AbortController();
  bar.classList.add('workspace-tabs-bar');
  bar.innerHTML = '<div class="workspace-tabs-scroll" role="tablist" aria-label="Open workspaces"></div><button class="workspace-tabs-more" type="button" aria-label="All open tabs" aria-haspopup="menu" aria-expanded="false" data-tooltip="All open tabs">⌄</button><div class="workspace-tabs-menu" role="menu" aria-label="All open tabs" hidden></div>';
  const strip = bar.querySelector('[role="tablist"]'), more = bar.querySelector('.workspace-tabs-more'), menu = bar.querySelector('[role="menu"]');
  const closeMenu = () => { menu.hidden = true; more.setAttribute('aria-expanded', 'false'); };
  function render() {
    if (disposed) return;
    const visible = tabs.filter(tab => !tab.hiddenTab);
    bar.hidden = visible.length === 0;
    const scroll = strip.scrollLeft;
    const focused = strip.contains(document.activeElement) ? document.activeElement.closest('[data-tab-id]')?.dataset.tabId : null;
    strip.innerHTML = visible.map(tab => `<div class="workspace-tab${tab.id === active ? ' is-active' : ''}${tab.closing ? ' is-closing' : ''}" data-tab-id="${escapeHtml(tab.id)}" draggable="${tab.id !== 'overview'}">
      <button type="button" role="tab" id="tab-${tab.domId}" aria-controls="panel-${tab.domId}" aria-selected="${tab.id === active}" tabindex="${tab.id === active || (!visible.some(entry => entry.id === active) && visible[0] === tab) ? 0 : -1}" data-tab-select="${escapeHtml(tab.id)}" title="${escapeHtml(tab.title)}${tab.status ? ` · ${escapeHtml(tab.status)}` : ''}">${symbol(tab.type)}<span>${escapeHtml(tab.title)}</span>${tab.status ? `<i class="workspace-tab-status is-${escapeHtml(tab.status.replace(/[^a-z_]/gi,''))}" aria-label="${escapeHtml(tab.status)}"></i>` : ''}</button>
      ${tab.closable === false ? '' : `<button type="button" class="workspace-tab-close" data-tab-close="${escapeHtml(tab.id)}" aria-label="${tab.type === 'ssh' ? 'Disconnect SSH and close' : 'Close'} ${escapeHtml(tab.title)}" data-tooltip="${tab.type === 'ssh' ? 'Disconnect SSH and close tab' : tab.type === 'local' ? 'Close local terminal' : 'Close tab; Plan continues in Agent'}" ${tab.closing ? 'disabled' : ''}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg></button>`}
    </div>`).join('');
    strip.scrollLeft = scroll;
    more.setAttribute('aria-label', `All open tabs (${visible.length})`);
    more.dataset.tooltip = `All open tabs (${visible.length})`;
    more.textContent = `${visible.length} ⌄`;
    menu.innerHTML = visible.map(tab => {
      const type = tab.type === 'local' ? 'Local terminal' : tab.type.toUpperCase();
      const endpoint = tab.host ? `${tab.host}${tab.port ? `:${tab.port}` : ''}` : '';
      const detail = [endpoint, type].filter(Boolean).join(' · ');
      return `<button type="button" role="menuitem" data-tab-select="${escapeHtml(tab.id)}" title="${escapeHtml(tab.title)}${detail ? ` · ${escapeHtml(detail)}` : ''}">${symbol(tab.type)}<span class="workspace-tab-menu-label"><strong>${escapeHtml(tab.title)}</strong><small>${escapeHtml(detail)}</small></span>${tab.status ? `<i class="workspace-tab-status is-${escapeHtml(tab.status.replace(/[^a-z_]/gi,''))}" aria-label="${escapeHtml(tab.status)}"></i>` : ''}</button>`;
    }).join('');
    if (focused) [...strip.querySelectorAll('[data-tab-select]')].find(button => button.dataset.tabSelect === focused)?.focus({ preventScroll: true });
  }
  function activate(id, { focusContent = true } = {}) {
    const tab = tabs.find(tab => tab.id === id);
    if (!tab || disposed) return;
    active = id;
    for (const entry of tabs) entry.pane.hidden = entry.id !== id;
    if (!tab.mounted) {
      tab.mounted = true;
      try { tab.lifecycle = tab.mount?.(tab.pane); }
      catch (error) { onError(String(error)); }
    }
    render(); closeMenu(); onActivate(tab);
    requestAnimationFrame(() => {
      if (active !== id || disposed) return;
      if (focusContent) tab.lifecycle?.activate?.();
      [...strip.querySelectorAll('[data-tab-select]')].find(button => button.dataset.tabSelect === id)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    });
  }
  function open(descriptor, { select = true, replace = false, eager = false } = {}) {
    let tab = tabs.find(tab => tab.id === descriptor.id);
    if (tab && replace) {
      tab.lifecycle?.dispose?.(); tab.pane.remove();
      tab.mounted = false; tab.lifecycle = null; tab.pane = paneFor(tab);
    }
    if (!tab) {
      tab = { ...descriptor, domId: crypto.randomUUID(), mounted: false };
      tab.pane = paneFor(tab); tabs.push(tab);
    } else Object.assign(tab, descriptor);
    if (select) activate(tab.id);
    else {
      if (eager && !tab.mounted) { tab.mounted = true; tab.lifecycle = tab.mount?.(tab.pane); }
      render();
    }
    return tab;
  }
  function paneFor(tab) {
    const pane = document.createElement('div');
    pane.id = `panel-${tab.domId}`; pane.className = `workspace-pane is-${tab.type}`;
    pane.setAttribute('role', tab.hiddenTab ? 'region' : 'tabpanel');
    if (tab.hiddenTab) pane.setAttribute('aria-label', tab.title);
    else pane.setAttribute('aria-labelledby', `tab-${tab.domId}`);
    pane.hidden = true; panels.append(pane); return pane;
  }
  function remove(id) {
    const tab = tabs.find(tab => tab.id === id);
    if (!tab || tab.closable === false) return;
    const visible = tabs.filter(entry => !entry.hiddenTab);
    const next = visible.length > 1 ? nextTabAfterClose(visible, id) : tabs.find(entry => entry.hiddenTab && entry.id === 'workspace')?.id ?? nextTabAfterClose(tabs, id);
    tab.lifecycle?.dispose?.();
    // Invalidate legacy view pollers before removing their DOM.
    tab.pane.replaceChildren(); tab.pane.remove();
    tabs = tabs.filter(entry => entry !== tab);
    if (active === id) activate(next); else render();
  }
  async function close(id) {
    const tab = tabs.find(tab => tab.id === id);
    if (!tab || tab.closable === false || tab.closing) return;
    tab.closing = true; render();
    try { await tab.onClose?.(); remove(id); }
    catch (error) { tab.closing = false; render(); onError(String(error)); }
  }
  function update(id, changes) {
    const tab = tabs.find(tab => tab.id === id);
    if (!tab || Object.entries(changes).every(([key, value]) => tab[key] === value)) return;
    Object.assign(tab, changes); render();
  }
  bar.addEventListener('click', event => {
    const closing = event.target.closest('[data-tab-close]');
    if (closing) { void close(closing.dataset.tabClose); return; }
    const button = event.target.closest('[data-tab-select]');
    if (button) activate(button.dataset.tabSelect);
  }, { signal: lifetime.signal });
  strip.addEventListener('keydown', event => {
    const button = event.target.closest('[role="tab"]');
    if (!button) return;
    const visible = tabs.filter(tab => !tab.hiddenTab);
    const index = visible.findIndex(tab => tab.id === button.dataset.tabSelect);
    const destination = { ArrowRight: (index + 1) % visible.length, ArrowLeft: (index + visible.length - 1) % visible.length, Home: 0, End: visible.length - 1 }[event.key];
    if (destination == null) return;
    event.preventDefault(); activate(visible[destination].id, { focusContent: false });
    [...strip.querySelectorAll('[role="tab"]')].find(element => element.dataset.tabSelect === active)?.focus();
  }, { signal: lifetime.signal });
  strip.addEventListener('dragstart', event => {
    const tab = event.target.closest('[data-tab-id]');
    if (!tab || tab.dataset.tabId === 'overview') { event.preventDefault(); return; }
    dragged = tab.dataset.tabId; event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('application/x-gatetree-ops-workspace-tab', dragged);
  }, { signal: lifetime.signal });
  strip.addEventListener('dragover', event => { if (dragged) event.preventDefault(); }, { signal: lifetime.signal });
  strip.addEventListener('drop', event => {
    if (!dragged) return;
    const target = event.target.closest('[data-tab-id]');
    event.preventDefault();
    if (target) {
      const rect = target.getBoundingClientRect();
      tabs = moveTab(tabs, dragged, target.dataset.tabId, event.clientX > rect.left + rect.width / 2);
    } else { const last = tabs.filter(tab => !tab.hiddenTab).at(-1); if (last) tabs = moveTab(tabs, dragged, last.id, true); }
    dragged = null; render();
  }, { signal: lifetime.signal });
  strip.addEventListener('dragend', () => { dragged = null; }, { signal: lifetime.signal });
  more.addEventListener('click', () => { menu.hidden = !menu.hidden; more.setAttribute('aria-expanded', String(!menu.hidden)); }, { signal: lifetime.signal });
  document.addEventListener('pointerdown', event => { if (!bar.contains(event.target)) closeMenu(); }, { signal: lifetime.signal });
  bar.addEventListener('keydown', event => { if (event.key === 'Escape') { closeMenu(); more.focus(); } }, { signal: lifetime.signal });
  return { open, activate, remove, close, update, activeId: () => active, get: id => tabs.find(tab => tab.id === id), entries: () => tabs.slice(),
    destroy() { disposed = true; lifetime.abort(); for (const tab of tabs) { tab.lifecycle?.dispose?.(); tab.pane.replaceChildren(); tab.pane.remove(); } tabs = []; bar.replaceChildren(); } };
}
