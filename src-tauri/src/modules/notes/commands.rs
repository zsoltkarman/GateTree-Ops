use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{Note, NoteFolder, NotesSnapshot},
    service::NotesService,
};

#[tauri::command]
pub fn list_notes(state: State<'_, WorkspaceState>) -> Result<NotesSnapshot, String> {
    NotesService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_note_folder(
    state: State<'_, WorkspaceState>,
    name: String,
) -> Result<NoteFolder, String> {
    NotesService::create_folder(&state, name).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn rename_note_folder(
    state: State<'_, WorkspaceState>,
    id: String,
    name: String,
) -> Result<NoteFolder, String> {
    NotesService::rename_folder(&state, id, name).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_note_folder(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    NotesService::delete_folder(&state, id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_note(
    state: State<'_, WorkspaceState>,
    folder_id: Option<String>,
) -> Result<Note, String> {
    NotesService::create_note(&state, folder_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_note(
    state: State<'_, WorkspaceState>,
    id: String,
    title: String,
    body: String,
) -> Result<Note, String> {
    NotesService::update_note(&state, id, title, body).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn move_note(
    state: State<'_, WorkspaceState>,
    id: String,
    folder_id: Option<String>,
) -> Result<Note, String> {
    NotesService::move_note(&state, id, folder_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_note(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    NotesService::delete_note(&state, id).map_err(|error| error.to_string())
}
