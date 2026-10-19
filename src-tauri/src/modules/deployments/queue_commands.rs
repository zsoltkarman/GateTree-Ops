use tauri::State;
use zeroize::Zeroizing;

use crate::{agent_ipc, modules::workspace::WorkspaceState};

use super::{queue_model::QueueSnapshot, queue_service::QueueService};

#[tauri::command]
pub fn get_deployment_queue(state: State<'_, WorkspaceState>) -> Result<QueueSnapshot, String> {
    QueueService::synchronize_agent(&state).map_err(|error| error.to_string())?;
    QueueService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_deployment_plan(
    state: State<'_, WorkspaceState>,
    definition: String,
) -> Result<QueueSnapshot, String> {
    QueueService::create(&state, definition).map_err(|error| error.to_string())?;
    QueueService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_deployment_plan(
    state: State<'_, WorkspaceState>,
    id: String,
    definition: String,
) -> Result<QueueSnapshot, String> {
    QueueService::update(&state, &id, definition).map_err(|error| error.to_string())?;
    QueueService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_deployment_plan(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<QueueSnapshot, String> {
    QueueService::delete(&state, &id).map_err(|error| error.to_string())?;
    QueueService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn control_deployment_queue_item(
    state: State<'_, WorkspaceState>,
    item_type: String,
    id: String,
    action: String,
) -> Result<QueueSnapshot, String> {
    let keepass_password = if matches!(action.as_str(), "start" | "restart") {
        let status = agent_ipc::status().map_err(|error| error.to_string())?;
        crate::modules::vaults::session_keepass_password(
            &state,
            status.pid.ok_or("Agent is not running")?,
        )?
    } else {
        None
    };
    if matches!(action.as_str(), "start" | "restart") && keepass_password.is_none() {
        return Err("Unlock KeePass for the current Agent session first.".into());
    }
    if item_type == "plan" {
        match action.as_str() {
            "start" | "restart" => {
                agent_ipc::start().map_err(|error| error.to_string())?;
                QueueService::synchronize_agent(&state).map_err(|error| error.to_string())?;
                QueueService::prepare_plan_start(&state, &id, action == "restart")
                    .map_err(|error| error.to_string())?;
                let submission = QueueService::agent_submission(
                    &state,
                    &id,
                    keepass_password.as_deref().map(String::as_str),
                )
                .map_err(|error| error.to_string())?;
                if submission
                    .jobs
                    .iter()
                    .any(|job| matches!(job.driver.as_str(), "jira" | "cx" | "olam"))
                    && submission.credentials.is_empty()
                {
                    return Err(
                        "KeePass master password is required for validation and deployment.".into(),
                    );
                }
                let submitted = agent_ipc::submit_job_set(submission, action == "restart")
                    .map_err(|error| error.to_string())?;
                QueueService::synchronize_job_sets(&state, &[submitted])
                    .map_err(|error| error.to_string())?;
                return QueueService::snapshot(&state).map_err(|error| error.to_string());
            }
            "stop" => {
                let stopped =
                    agent_ipc::stop_job_set(id.clone()).map_err(|error| error.to_string())?;
                QueueService::synchronize_job_sets(&state, &[stopped])
                    .map_err(|error| error.to_string())?;
                return QueueService::snapshot(&state).map_err(|error| error.to_string());
            }
            _ => {}
        }
    }
    if item_type == "job" && matches!(action.as_str(), "disable" | "enable") {
        // Existing Agent jobs are authoritative, including queued jobs in a running Plan.
        let status = agent_ipc::status().map_err(|error| error.to_string())?;
        if status.running {
            let sets = agent_ipc::list_job_sets().map_err(|error| error.to_string())?;
            if sets
                .iter()
                .any(|set| set.jobs.iter().any(|job| job.id == id))
            {
                let updated = agent_ipc::set_job_enabled(id, action == "enable")
                    .map_err(|error| error.to_string())?;
                QueueService::synchronize_job_sets(&state, &[updated])
                    .map_err(|error| error.to_string())?;
                return QueueService::snapshot(&state).map_err(|error| error.to_string());
            }
        }
        QueueService::control(&state, "job", &id, &action).map_err(|error| error.to_string())?;
        return QueueService::snapshot(&state).map_err(|error| error.to_string());
    }
    if item_type == "job" && matches!(action.as_str(), "start" | "restart" | "stop") {
        let snapshot = QueueService::snapshot(&state).map_err(|error| error.to_string())?;
        let plan = snapshot
            .plans
            .iter()
            .find(|plan| plan.jobs.iter().any(|job| job.id == id))
            .ok_or_else(|| "Queue job does not exist.".to_string())?;
        if action == "stop" {
            agent_ipc::stop_job_set(plan.id.clone()).map_err(|error| error.to_string())?;
        } else {
            let submission = QueueService::agent_submission(
                &state,
                &plan.id,
                keepass_password.as_deref().map(String::as_str),
            )
            .map_err(|error| error.to_string())?;
            agent_ipc::retry_job(id.clone(), submission.credentials)
                .map_err(|error| error.to_string())?;
        }
        QueueService::synchronize_agent(&state).map_err(|error| error.to_string())?;
        return QueueService::snapshot(&state).map_err(|error| error.to_string());
    }
    if item_type == "job" && action == "deny" {
        agent_ipc::deny_job(id.clone()).map_err(|error| error.to_string())?;
        QueueService::synchronize_agent(&state).map_err(|error| error.to_string())?;
        return QueueService::snapshot(&state).map_err(|error| error.to_string());
    }
    if item_type == "job" && action == "approve" {
        let approved = agent_ipc::approve_job(id).map_err(|error| error.to_string())?;
        QueueService::synchronize_job_sets(&state, &[approved])
            .map_err(|error| error.to_string())?;
        return QueueService::snapshot(&state).map_err(|error| error.to_string());
    }
    QueueService::control(&state, &item_type, &id, &action).map_err(|error| error.to_string())?;
    QueueService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn restore_deployment_credentials(
    state: State<'_, WorkspaceState>,
    keepass_password: String,
) -> Result<(), String> {
    let password = Zeroizing::new(keepass_password);
    if password.is_empty() {
        return Err("KeePass master password is required.".into());
    }
    let database_path = crate::modules::vaults::validate_workspace_keepass(&state, &password)?;
    let agent = agent_ipc::start().map_err(|error| error.to_string())?;
    let sets = agent_ipc::list_job_sets().map_err(|error| error.to_string())?;
    // Resolve every account before updating the Agent, so a wrong password can be retried.
    let snapshot = QueueService::snapshot(&state).map_err(|error| error.to_string())?;
    let mut restored = Vec::new();
    for set in sets.into_iter().filter(|set| {
        set.credentials_required && snapshot.plans.iter().any(|plan| plan.id == set.id)
    }) {
        restored.push((
            set.id,
            QueueService::credentials_for_agent_set(
                &state,
                &set.plan_definition,
                &set.process_definition,
                &set.jobs,
                &password,
            )
            .map_err(|error| error.to_string())?,
        ));
    }
    for (id, credentials) in restored {
        agent_ipc::restore_job_set_credentials(id, credentials)
            .map_err(|error| error.to_string())?;
    }
    let current = agent_ipc::status().map_err(|error| error.to_string())?;
    if current.pid != agent.pid {
        return Err("Agent restarted during KeePass unlock. Unlock again.".into());
    }
    state
        .cache_keepass_password(
            agent.pid.ok_or("Agent is not running")?,
            database_path,
            password,
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
