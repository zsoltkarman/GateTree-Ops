// GateTree's twelve URL icon choices and palette, rendered as portable SVG.
export const URL_ICONS = [
  { id: 'icon1', label: 'Diamond', color: '#0052c7', path: '<path fill="currentColor" stroke="none" d="m12 2 10 10-10 10L2 12Z"/>' },
  { id: 'icon2', label: 'List', color: '#1775e3', path: '<path d="M4 5h16M4 9h11M4 13h16M4 17h11"/>' },
  { id: 'icon3', label: 'Panels', color: '#13a6c2', path: '<rect x="2" y="6" width="5" height="12" rx="1"/><rect x="9.5" y="4" width="5" height="16" rx="1"/><rect x="17" y="6" width="5" height="12" rx="1"/>' },
  { id: 'icon4', label: 'Code', color: '#7362d6', path: '<path d="m7 6-5 6 5 6M17 6l5 6-5 6M14 3 10 21"/>' },
  { id: 'icon5', label: 'Alerts', color: '#e5484d', path: '<path d="M18 9v5l2 3H4l2-3V9a6 6 0 0 1 9-5M10 21h4"/><circle cx="19" cy="5" r="3" fill="currentColor" stroke="none"/>' },
  { id: 'icon6', label: 'Gauge', color: '#e58419', path: '<path d="M4 19a10 10 0 1 1 16 0M5 11l2 1M9 5l1 2M17 6l-1 2M20 12h-2M12 15l4-5"/><circle cx="12" cy="15" r="1.5" fill="currentColor"/>' },
  { id: 'icon7', label: 'Flame', color: '#d12e14', path: '<path d="M13 2c1 6-5 7-4 11 2-1 3-3 3-3 5 3 7 6 5 10 5-2 6-8 1-13 0 3-2 4-2 4 1-4-1-7-3-9ZM8 9C0 17 6 23 12 22 7 20 5 16 8 9Z" fill="currentColor" stroke="none"/>' },
  { id: 'icon8', label: 'Verified', color: '#a64ac9', path: '<path d="m12 2 3 2 4 1 1 4 2 3-2 3-1 4-4 1-3 2-3-2-4-1-1-4-2-3 2-3 1-4 4-1Z"/><path d="m7 12 3 3 7-7"/>' },
  { id: 'icon9', label: 'Chart', color: '#b049ca', path: '<path d="M3 3v18h18M6 15l5-5 4 3 6-8"/><circle cx="11" cy="10" r="1" fill="currentColor"/><circle cx="15" cy="13" r="1" fill="currentColor"/>' },
  { id: 'icon10', label: 'Branch', color: '#159c9e', path: '<circle cx="6" cy="4" r="2"/><circle cx="6" cy="20" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 6v12M18 7v3c0 5-12 2-12 7"/>' },
  { id: 'icon11', label: 'Help', color: '#89919d', path: '<circle cx="12" cy="12" r="9"/><path d="M9 9a3 3 0 1 1 5 2c-2 1-2 2-2 3M12 18h.01"/>' },
  { id: 'icon12', label: 'Globe', color: '', path: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c5 5 5 13 0 18-5-5-5-13 0-18Z"/>' },
];
const types = {
  ssh: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>',
  rdp: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>',
};
export function connectionIcon(type, icon = 'icon12') {
  const choice = URL_ICONS.find(item => item.id === icon) || URL_ICONS[11];
  const color = type === 'url' ? choice.color : '';
  return `<svg class="connection-type-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"${color ? ` style="color:${color}"` : ''} aria-hidden="true">${types[type] || choice.path}</svg>`;
}
export function connectionIconPicker(value = 'icon12') {
  return `<fieldset class="connection-icon-picker"><legend>Icon</legend><div>${URL_ICONS.map(icon => `<label title="${icon.label}"><input type="radio" name="connection-icon" value="${icon.id}" aria-label="${icon.label}"${value === icon.id ? ' checked' : ''}>${connectionIcon('url', icon.id)}</label>`).join('')}</div></fieldset>`;
}
