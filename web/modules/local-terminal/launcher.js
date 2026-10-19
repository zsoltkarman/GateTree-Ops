import { TERMINAL_THEMES } from './themes.js';
export function terminalLauncherMarkup() {
  return `<div class="local-terminal-launcher"><button type="button" data-action="local-terminal-menu" aria-label="Open local terminal" data-tooltip="Open local terminal" aria-haspopup="menu" aria-expanded="false"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3m5 0h5"/></svg><svg class="terminal-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="m3 4 3 3 3-3"/></svg></button><div class="local-terminal-menu" role="menu" aria-label="Terminal color scheme" hidden>${TERMINAL_THEMES.map(theme => `<button type="button" role="menuitem" data-terminal-theme="${theme.id}"><span class="local-terminal-swatch" aria-hidden="true" style="--terminal-background:${theme.background};--terminal-foreground:${theme.foreground}"></span>${theme.name}</button>`).join('')}</div></div>`;
}
export function mountTerminalLauncher(root, onOpen, onError = () => {}) {
  const host = root.querySelector('.local-terminal-launcher');
  const button = host.querySelector('[aria-haspopup]'), menu = host.querySelector('[role="menu"]');
  const lifetime = new AbortController(), options = { signal: lifetime.signal };
  const close = () => { menu.hidden = true; button.setAttribute('aria-expanded', 'false'); };
  const items = () => [...menu.querySelectorAll('[role="menuitem"]')];
  const open = () => { menu.hidden = false; button.setAttribute('aria-expanded', 'true'); items()[0].focus(); };
  button.addEventListener('click', () => menu.hidden ? open() : close(), options);
  button.addEventListener('keydown', event => { if (event.key === 'ArrowDown') { event.preventDefault(); event.stopPropagation(); open(); } }, options);
  menu.addEventListener('click', event => {
    const theme = event.target.closest('[data-terminal-theme]')?.dataset.terminalTheme;
    if (!theme) return;
    close(); button.focus();
    Promise.resolve().then(() => onOpen(theme)).catch(onError);
  }, options);
  host.addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); close(); button.focus(); }
    if (!menu.hidden && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault(); const all = items(), i = all.indexOf(document.activeElement);
      all[event.key === 'Home' ? 0 : event.key === 'End' ? all.length-1 : (i + (event.key === 'ArrowDown' ? 1 : -1) + all.length) % all.length].focus();
    }
  }, options);
  document.addEventListener('pointerdown', event => { if (!host.contains(event.target)) close(); }, options);
  document.addEventListener('focusin', event => { if (!host.contains(event.target)) close(); }, options);
  return () => lifetime.abort();
}
