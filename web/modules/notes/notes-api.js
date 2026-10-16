const { invoke } = window.__TAURI__.core;

export const listNotes = () => invoke("list_notes");
export const createNoteFolder = (name) => invoke("create_note_folder", { name });
export const renameNoteFolder = (id, name) => invoke("rename_note_folder", { id, name });
export const deleteNoteFolder = (id) => invoke("delete_note_folder", { id });
export const createNote = (folderId) => invoke("create_note", { folderId });
export const updateNote = (id, title, body) => invoke("update_note", { id, title, body });
export const moveNote = (id, folderId) => invoke("move_note", { id, folderId });
export const deleteNote = (id) => invoke("delete_note", { id });
