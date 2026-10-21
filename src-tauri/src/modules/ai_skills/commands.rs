use tauri::{State, ipc::Channel};

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
pub async fn run_ai_skill(
    state: State<'_, WorkspaceState>,
    id: String,
    input: String,
    on_output: Channel<String>,
) -> Result<AiSkillRunResult, String> {
    let command =
        AiSkillsService::prepare_skill_run(&state, id, input).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        super::service::execute_skill(command, |chunk| {
            let _ = on_output.send(chunk.to_owned());
        })
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
