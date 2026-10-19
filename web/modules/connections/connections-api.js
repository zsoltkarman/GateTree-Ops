const { invoke } = window.__TAURI__.core;
const change = async (command, args) => {
  const result = await invoke(command, args);
  window.dispatchEvent(new Event("gatetree-ops:connections-changed"));
  return result;
};

export const listConnectionFolders = () => invoke("list_connection_folders");
export const createConnectionFolder = (name, parentId) => change("create_connection_folder", { name, parentId });
export const renameConnectionFolder = (id, name) => change("rename_connection_folder", { id, name });
export const moveConnectionFolder = (id, parentId) => change("move_connection_folder", { id, parentId });
export const deleteConnectionFolder = (id) => change("delete_connection_folder", { id });
export const listConnectionItems = () => invoke("list_connection_items");
export const createConnectionItem = (folderId, connectionType, name, target, port, tag, username, authentication, credentialReference, sshTransport, icon = null, credentialMode = null) => change("create_connection_item", { folderId, connectionType, name, target, port, tag, username, authentication, credentialReference, sshTransport, icon, credentialMode });
export const moveConnectionItem = (id, folderId) => change("move_connection_item", { id, folderId });
export const updateConnectionItem = (id, name, target, port, tag, username, authentication, credentialReference, sshTransport, icon = null, credentialMode = null) => change("update_connection_item", { id, name, target, port, tag, username, authentication, credentialReference, sshTransport, icon, credentialMode });

export const setConnectionFolderCredentials = (id, mode, credentialReference) => change("set_connection_folder_credentials", { id, mode, credentialReference });
export const resolveConnectionAccount = connectionId => invoke("resolve_connection_account", { connectionId });
