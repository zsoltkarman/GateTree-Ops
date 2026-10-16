const { invoke } = window.__TAURI__.core;

export const getVaults = () => invoke("get_vaults");
export const setKeePassDatabasePath = (path) => invoke("set_keepass_database_path", { path });
export const validateAndSetKeePassDatabasePath = (path, masterPassword) =>
  invoke("validate_and_set_keepass_database_path", { path, masterPassword });
export const clearKeePassDatabasePath = () => invoke("clear_keepass_database_path");
export const createVaultEntry = (name, entryPath) => invoke("create_vault_entry", { name, entryPath });
export const updateVaultEntry = (id, name, entryPath) => invoke("update_vault_entry", { id, name, entryPath });
export const deleteVaultEntry = (id) => invoke("delete_vault_entry", { id });
