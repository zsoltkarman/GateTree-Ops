const { invoke } = window.__TAURI__.core;

export const inspectWorkspace = (path) => invoke("inspect_workspace", { path });
export const createWorkspace = (path, password) => invoke("create_workspace", { path, password });
export const unlockWorkspace = (path, password) => invoke("unlock_workspace", { path, password });
export const lockWorkspace = () => invoke("lock_workspace");
export const biometricStatus = (path) => invoke("biometric_status", { path });
export const enableBiometricUnlock = () => invoke("enable_biometric_unlock");
export const disableBiometricUnlock = (path) => invoke("disable_biometric_unlock", { path });
export const unlockWorkspaceWithBiometrics = (path) => invoke("unlock_workspace_with_biometrics", { path });
export const unlockWorkspaceForDevelopment = (path) => invoke("unlock_workspace_for_development", { path });

