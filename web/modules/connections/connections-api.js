const { invoke } = window.__TAURI__.core;

export const listConnectionFolders = () => invoke("list_connection_folders");
export const createConnectionFolder = (name, parentId) => invoke("create_connection_folder", { name, parentId });
export const renameConnectionFolder = (id, name) => invoke("rename_connection_folder", { id, name });
export const moveConnectionFolder = (id, parentId) => invoke("move_connection_folder", { id, parentId });
export const deleteConnectionFolder = (id) => invoke("delete_connection_folder", { id });
export const listConnectionItems = () => invoke("list_connection_items");
export const createConnectionItem = (folderId, connectionType, name, target, port, tag, username, authentication, credentialReference, sshTransport) => invoke("create_connection_item", { folderId, connectionType, name, target, port, tag, username, authentication, credentialReference, sshTransport });
export const moveConnectionItem = (id, folderId) => invoke("move_connection_item", { id, folderId });
export const updateConnectionItem = (id, name, target, port, tag, username, authentication, credentialReference, sshTransport) => invoke("update_connection_item", { id, name, target, port, tag, username, authentication, credentialReference, sshTransport });
