export const APP_NAME = "GateTree-Ops";

// Copy settings within this app's origin; never replace an existing new setting.
export function migrateLegacyPreferences(storage = localStorage) {
  const legacyPrefix = "pharos.";
  const entries = Array.from({ length: storage.length }, (_, index) => storage.key(index))
    .filter((key) => key?.startsWith(legacyPrefix));
  for (const oldKey of entries) {
    const key = `gatetree-ops.${oldKey.slice(legacyPrefix.length)}`;
    if (storage.getItem(key) === null) storage.setItem(key, storage.getItem(oldKey));
  }
}
