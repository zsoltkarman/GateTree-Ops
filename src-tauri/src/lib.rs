pub use pharos_agent_core as agent_ipc;
mod modules;
mod platform;
mod storage;

use modules::agent::{
    agent_status, connect_agent_ssh_connection, delete_agent_activity,
    disconnect_agent_ssh_session, focus_url_activity, forget_agent_ssh_host_key,
    inspect_agent_ssh_host_key, kill_agent, list_agent_activity, list_agent_job_sets,
    list_agent_ssh_sessions, open_url_connection, read_agent_ssh_terminal,
    resize_agent_ssh_terminal, scan_agent_ssh_host_key, start_agent, stop_agent,
    trust_agent_ssh_host_key, write_agent_ssh_terminal,
};
use modules::ai_skills::{
    create_ai_skill_library, delete_ai_skill_library, get_ai_skills, read_ai_skill,
    refresh_ai_skills, run_ai_skill, update_ai_skill_library,
};
use modules::backup::{export_backup, import_backup};
use modules::connections::{
    create_connection_folder, create_connection_item, delete_connection_folder,
    list_connection_folders, list_connection_items, move_connection_folder, move_connection_item,
    rename_connection_folder, update_connection_item,
};
use modules::deployments::{
    control_deployment_queue_item, create_deployment_plan, delete_deployment_config_resource,
    delete_deployment_plan, duplicate_deployment_config_resource, get_deployment_config,
    get_deployment_config_bundle, get_deployment_queue, restore_deployment_config_resource,
    restore_deployment_credentials, save_deployment_config_resource, update_deployment_plan,
    validate_deployment_config_resource,
};
use modules::notes::{
    create_note, create_note_folder, delete_note, delete_note_folder, list_notes, move_note,
    rename_note_folder, update_note,
};
use modules::vaults::{
    clear_keepass_database_path, create_vault_entry, delete_vault_entry, get_vaults,
    set_keepass_database_path, update_vault_entry, validate_and_set_keepass_database_path,
};
use modules::workspace::{
    WorkspaceState, biometric_status, create_workspace, disable_biometric_unlock,
    enable_biometric_unlock, inspect_workspace, lock_workspace, unlock_workspace,
    unlock_workspace_for_development, unlock_workspace_with_biometrics,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(WorkspaceState::default())
        .invoke_handler(tauri::generate_handler![
            inspect_workspace,
            create_workspace,
            unlock_workspace,
            lock_workspace,
            biometric_status,
            enable_biometric_unlock,
            disable_biometric_unlock,
            unlock_workspace_with_biometrics,
            unlock_workspace_for_development,
            list_notes,
            create_note_folder,
            rename_note_folder,
            delete_note_folder,
            create_note,
            update_note,
            move_note,
            delete_note,
            get_vaults,
            set_keepass_database_path,
            validate_and_set_keepass_database_path,
            clear_keepass_database_path,
            create_vault_entry,
            update_vault_entry,
            delete_vault_entry,
            export_backup,
            import_backup,
            list_connection_folders,
            create_connection_folder,
            rename_connection_folder,
            move_connection_folder,
            delete_connection_folder,
            list_connection_items,
            create_connection_item,
            move_connection_item,
            update_connection_item,
            start_agent,
            stop_agent,
            kill_agent,
            agent_status,
            scan_agent_ssh_host_key,
            inspect_agent_ssh_host_key,
            trust_agent_ssh_host_key,
            forget_agent_ssh_host_key,
            list_agent_ssh_sessions,
            list_agent_job_sets,
            connect_agent_ssh_connection,
            disconnect_agent_ssh_session,
            read_agent_ssh_terminal,
            write_agent_ssh_terminal,
            resize_agent_ssh_terminal,
            list_agent_activity,
            open_url_connection,
            focus_url_activity,
            delete_agent_activity,
            get_ai_skills,
            create_ai_skill_library,
            update_ai_skill_library,
            delete_ai_skill_library,
            refresh_ai_skills,
            read_ai_skill,
            run_ai_skill,
            get_deployment_config,
            validate_deployment_config_resource,
            save_deployment_config_resource,
            delete_deployment_config_resource,
            duplicate_deployment_config_resource,
            restore_deployment_config_resource,
            get_deployment_config_bundle,
            get_deployment_queue,
            restore_deployment_credentials,
            create_deployment_plan,
            update_deployment_plan,
            delete_deployment_plan,
            control_deployment_queue_item
        ])
        .run(tauri::generate_context!())
        .expect("error while running PHAROS");
}
