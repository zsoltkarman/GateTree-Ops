use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{DeploymentConfigResource, DeploymentConfigSnapshot, DeploymentConfigValidation},
    service::DeploymentsService,
};

#[tauri::command]
pub fn get_deployment_config(
    state: State<'_, WorkspaceState>,
) -> Result<DeploymentConfigSnapshot, String> {
    DeploymentsService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn validate_deployment_config_resource(
    state: State<'_, WorkspaceState>,
    resource_type: String,
    definition: String,
    replacing_id: Option<String>,
) -> Result<DeploymentConfigValidation, String> {
    DeploymentsService::validate(&state, &resource_type, definition, replacing_id.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_deployment_config_resource(
    state: State<'_, WorkspaceState>,
    resource_type: String,
    definition: String,
    replacing_id: Option<String>,
) -> Result<DeploymentConfigResource, String> {
    DeploymentsService::save(&state, &resource_type, definition, replacing_id.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_deployment_config_resource(
    state: State<'_, WorkspaceState>,
    resource_type: String,
    id: String,
) -> Result<(), String> {
    DeploymentsService::delete(&state, &resource_type, &id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn duplicate_deployment_config_resource(
    state: State<'_, WorkspaceState>,
    resource_type: String,
    id: String,
) -> Result<DeploymentConfigResource, String> {
    DeploymentsService::duplicate(&state, &resource_type, &id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn restore_deployment_config_resource(
    state: State<'_, WorkspaceState>,
    resource_type: String,
    id: String,
) -> Result<DeploymentConfigResource, String> {
    DeploymentsService::restore_previous(&state, &resource_type, &id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_deployment_config_bundle(state: State<'_, WorkspaceState>) -> Result<String, String> {
    DeploymentsService::bundle(&state).map_err(|error| error.to_string())
}
