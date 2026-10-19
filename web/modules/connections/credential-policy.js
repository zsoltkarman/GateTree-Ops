// Only account references and labels belong in connection configuration.
export function credentialMode(item) {
  return item?.credentialMode ?? (item?.credentialReference ? 'keepass' : 'inherit');
}
export function resolveCredentialPolicy(item, folders, entries, { folder = false } = {}) {
  if (!folder && item.connectionType === 'url') return { mode: 'manual', account: null, source: null };
  let current = item, mode = credentialMode(current), source = folder ? item : null;
  const seen = new Set();
  while (mode === 'inherit') {
    const id = current === item && !folder ? item.folderId : current?.parentId;
    if (!id || seen.has(id)) return { mode:'manual', account:null, source:null };
    seen.add(id);
    current = folders.find(candidate => candidate.id === id);
    if (!current) return { mode:'manual', account:null, source:null };
    source = current;
    mode = credentialMode(current);
  }
  return { mode, account: mode === 'keepass' ? entries.find(entry => entry.id === current.credentialReference) ?? null : null, source };
}
export function credentialSelection(item) {
  const mode = credentialMode(item);
  return mode === 'keepass' ? `keepass:${item.credentialReference ?? ''}` : mode;
}
export function parseCredentialSelection(value) {
  if (value.startsWith('keepass:')) return { mode:'keepass', reference:value.slice(8) || null };
  return { mode:value === 'manual' ? 'manual' : 'inherit', reference:null };
}
