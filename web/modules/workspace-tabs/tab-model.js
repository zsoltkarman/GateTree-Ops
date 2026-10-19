export function moveTab(tabs, id, beforeId, after = false) {
  if (id === 'overview' || id === beforeId) return tabs;
  const item = tabs.find(tab => tab.id === id);
  if (!item || !tabs.some(tab => tab.id === beforeId)) return tabs;
  const rest = tabs.filter(tab => tab.id !== id);
  const index = Math.max(1, rest.findIndex(tab => tab.id === beforeId) + (after ? 1 : 0));
  return [...rest.slice(0, index), item, ...rest.slice(index)];
}
export function nextTabAfterClose(tabs, id) {
  const index = tabs.findIndex(tab => tab.id === id);
  return tabs[index + 1]?.id ?? tabs[index - 1]?.id ?? 'overview';
}
