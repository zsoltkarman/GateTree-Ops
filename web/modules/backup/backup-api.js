const { invoke } = window.__TAURI__.core;

export const exportBackup = (path) => invoke("export_backup", { path });
export const importBackup = (path) => invoke("import_backup", { path });
