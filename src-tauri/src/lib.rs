pub use gatetree_ops_agent_core as agent_ipc;
mod modules;
mod platform;
mod storage;

use modules::agent::{
    agent_status, connect_agent_ssh_connection, connect_agent_ssh_with_keepass,
    delete_agent_activity, disconnect_agent_ssh_session, focus_url_activity,
    forget_agent_ssh_host_key, inspect_agent_ssh_host_key, kill_agent, list_agent_activity,
    list_agent_job_sets, list_agent_ssh_sessions, open_url_connection, read_agent_ssh_terminal,
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
    list_connection_folders, list_connection_items, list_quick_launch_items,
    move_connection_folder, move_connection_item, pin_quick_launch_item, remove_quick_launch_item,
    rename_connection_folder, rename_quick_launch_item, reorder_quick_launch_items,
    resolve_connection_account, set_connection_folder_credentials, update_connection_item,
};
use modules::deployments::{
    control_deployment_queue_item, create_deployment_plan, delete_deployment_config_resource,
    delete_deployment_plan, duplicate_deployment_config_resource, get_deployment_config,
    get_deployment_config_bundle, get_deployment_queue, restore_deployment_config_resource,
    restore_deployment_credentials, save_deployment_config_resource, update_deployment_plan,
    validate_deployment_config_resource,
};
use modules::local_terminal::{
    LocalTerminals, close_local_terminal, open_local_terminal, read_local_terminal,
    resize_local_terminal, write_local_terminal,
};
use modules::notes::{
    create_note, create_note_folder, delete_note, delete_note_folder, list_notes, move_note,
    rename_note_folder, update_note,
};
use modules::updates::{UpdateState, check_app_update, install_app_update};
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
    use tauri::Manager;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(UpdateState::default())
        .manage(LocalTerminals::default())
        .on_window_event(|window, event| {
            if window.label() == "main"
                && window.state::<UpdateState>().is_installing()
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
            }
        })
        .manage(WorkspaceState::default())
        .invoke_handler(tauri::generate_handler![
            open_local_terminal,
            read_local_terminal,
            write_local_terminal,
            resize_local_terminal,
            close_local_terminal,
            check_app_update,
            install_app_update,
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
            set_connection_folder_credentials,
            resolve_connection_account,
            create_connection_folder,
            rename_connection_folder,
            move_connection_folder,
            delete_connection_folder,
            list_connection_items,
            list_quick_launch_items,
            pin_quick_launch_item,
            rename_quick_launch_item,
            remove_quick_launch_item,
            reorder_quick_launch_items,
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
            connect_agent_ssh_with_keepass,
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
        .build(tauri::generate_context!())
        .expect("error while building GateTree-Ops")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<LocalTerminals>().clear();
            }
        });
}
