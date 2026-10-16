use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{
        AiSkillDocument, AiSkillLibrary, AiSkillRefreshResult, AiSkillRunResult, AiSkillsSnapshot,
    },
    service::AiSkillsService,
};

#[tauri::command]
pub fn get_ai_skills(state: State<'_, WorkspaceState>) -> Result<AiSkillsSnapshot, String> {
    AiSkillsService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_ai_skill_library(
    state: State<'_, WorkspaceState>,
    name: String,
    path: String,
) -> Result<AiSkillLibrary, String> {
    AiSkillsService::create_library(&state, name, path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_ai_skill_library(
    state: State<'_, WorkspaceState>,
    id: String,
    name: String,
    path: String,
) -> Result<AiSkillLibrary, String> {
    AiSkillsService::update_library(&state, id, name, path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_ai_skill_library(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    AiSkillsService::delete_library(&state, id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn refresh_ai_skills(state: State<'_, WorkspaceState>) -> Result<AiSkillRefreshResult, String> {
    AiSkillsService::refresh(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn read_ai_skill(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<AiSkillDocument, String> {
    AiSkillsService::read_skill(&state, id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn run_ai_skill(
    state: State<'_, WorkspaceState>,
    id: String,
    input: String,
) -> Result<AiSkillRunResult, String> {
    AiSkillsService::run_skill(&state, id, input).map_err(|error| error.to_string())
}
