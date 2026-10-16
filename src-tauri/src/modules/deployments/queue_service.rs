use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use serde_yaml::Value as YamlValue;
use thiserror::Error;
use uuid::Uuid;

use crate::agent_ipc::{AgentCredential, AgentIpcError, AgentJobSetSubmission, AgentJobSubmission};
use crate::modules::{
    vaults::read_keepass_entry,
    workspace::{WorkspaceError, WorkspaceState},
};
use crate::storage::StorageError;

use super::queue_model::{QueueJob, QueuePlan, QueueSnapshot};

pub struct QueueService;

#[derive(Debug, Deserialize)]
struct PlanInput {
    plan_name: String,
    process: String,
    inputs: PlanInputs,
}

#[derive(Debug, Deserialize)]
struct PlanInputs {
    #[serde(default)]
    jira: Option<JiraInput>,
    #[serde(default)]
    cx: Option<CxInput>,
    olam: OlamInput,
}

#[derive(Debug, Deserialize)]
struct JiraInput {
    issue: String,
}

#[derive(Debug, Deserialize)]
struct CxInput {
    request: String,
    #[serde(default, rename = "requester")]
    _requester: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OlamInput {
    strategy: String,
    account: String,
    targets: OlamTargets,
    steps: Vec<PlanStep>,
}

#[derive(Debug, Deserialize)]
struct OlamTargets {
    #[serde(default, rename = "sites")]
    _sites: Vec<String>,
    roles: Vec<String>,
    hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PlanStep {
    name: String,
    #[serde(default, rename = "extra_vars")]
    extra_vars: YamlValue,
}

struct JobDefinition {
    name: String,
    target: Option<String>,
    details: String,
    payload: Option<String>,
    driver: &'static str,
    action: &'static str,
}

impl QueueService {
    pub fn credentials_for_agent_set(
        state: &WorkspaceState,
        plan_definition: &str,
        process_definition: &str,
        jobs: &[crate::agent_ipc::AgentJobStatus],
        password: &str,
    ) -> Result<Vec<AgentCredential>, QueueError> {
        let jobs = jobs
            .iter()
            .map(|job| AgentJobSubmission {
                id: job.id.clone(),
                sequence: job.sequence,
                name: job.name.clone(),
                driver: job.driver.clone(),
                action: job.action.clone(),
                target: job.target.clone(),
                details: job.details.clone(),
                payload: job.payload.clone(),
            })
            .collect::<Vec<_>>();
        state
            .with_connection(|connection| {
                resolve_agent_credentials(
                    connection,
                    process_definition,
                    plan_definition,
                    &jobs,
                    Some(password),
                )
            })
            .map_err(map_workspace_error)
    }

    pub fn synchronize_agent(state: &WorkspaceState) -> Result<(), QueueError> {
        let Ok(job_sets) = crate::agent_ipc::list_job_sets() else {
            return Ok(());
        };
        state.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            for set in job_sets {
                transaction.execute(
                    "UPDATE deployment_plans SET status = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![set.id, set.status],
                )?;
                for job in set.jobs {
                    transaction.execute(
                        "UPDATE deployment_jobs SET status = ?2, log_output = ?3 WHERE id = ?1",
                        params![job.id, job.status, job.log],
                    )?;
                }
            }
            transaction.commit()
        }).map_err(QueueError::from)
    }

    pub fn snapshot(state: &WorkspaceState) -> Result<QueueSnapshot, QueueError> {
        state
            .with_connection(|connection| {
                upgrade_legacy_queued_plans(connection)?;
                Ok(QueueSnapshot {
                    plans: load_plans(connection)?,
                })
            })
            .map_err(map_workspace_error)
    }

    pub fn create(state: &WorkspaceState, definition: String) -> Result<(), QueueError> {
        let input: PlanInput = serde_yaml::from_str(&definition)?;
        validate_input(&input)?;
        state
            .with_connection(|connection| {
                validate_references(connection, &input).map_err(to_sql_error)?;
                let defaults = load_process_payload_defaults(connection, &input)
                    .map_err(to_sql_error)?;
                load_process_olam_account(connection, &input).map_err(to_sql_error)?;
                let jobs = generate_jobs(&input, &defaults).map_err(to_sql_error)?;
                let plan_id = Uuid::new_v4().to_string();
                let transaction = connection.unchecked_transaction()?;
                transaction.execute(
                    "UPDATE workspace_metadata
                     SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                     WHERE key = 'deployment_plan_sequence'",
                    [],
                )?;
                let plan_number: i64 = transaction.query_row(
                    "SELECT CAST(value AS INTEGER) FROM workspace_metadata
                     WHERE key = 'deployment_plan_sequence'",
                    [],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "INSERT INTO deployment_plans(id, plan_number, name, process_name, definition_yaml)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        plan_id,
                        plan_number,
                        input.plan_name.trim(),
                        input.process.trim(),
                        definition
                    ],
                )?;
                for (index, job) in jobs.into_iter().enumerate() {
                    transaction.execute(
                        "INSERT INTO deployment_jobs(id, plan_id, sequence, name, target, details, payload_json, driver, action)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            Uuid::new_v4().to_string(),
                            plan_id,
                            index as i64 + 1,
                            job.name,
                            job.target,
                            job.details,
                            job.payload,
                            job.driver,
                            job.action
                        ],
                    )?;
                }
                transaction.commit()
            })
            .map_err(map_workspace_error)
    }

    pub fn update(state: &WorkspaceState, id: &str, definition: String) -> Result<(), QueueError> {
        let input: PlanInput = serde_yaml::from_str(&definition)?;
        validate_input(&input)?;
        state
            .with_connection(|connection| {
                validate_references(connection, &input).map_err(to_sql_error)?;
                let status = connection
                    .query_row(
                        "SELECT status FROM deployment_plans WHERE id = ?1",
                        [id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                let Some(status) = status else {
                    return Err(to_sql_error(QueueError::Invalid(
                        "Queue Plan does not exist.".into(),
                    )));
                };
                if status == "running" {
                    return Err(to_sql_error(QueueError::Invalid(
                        "A running Queue Plan cannot be edited.".into(),
                    )));
                }

                let defaults =
                    load_process_payload_defaults(connection, &input).map_err(to_sql_error)?;
                load_process_olam_account(connection, &input).map_err(to_sql_error)?;
                let jobs = generate_jobs(&input, &defaults).map_err(to_sql_error)?;
                let transaction = connection.unchecked_transaction()?;
                transaction.execute(
                    "UPDATE deployment_plans
                     SET name = ?2, process_name = ?3, definition_yaml = ?4,
                         status = 'queued', updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![id, input.plan_name.trim(), input.process.trim(), definition],
                )?;
                transaction.execute("DELETE FROM deployment_jobs WHERE plan_id = ?1", [id])?;
                insert_jobs(&transaction, id, jobs)?;
                transaction.commit()
            })
            .map_err(map_workspace_error)
    }

    pub fn delete(state: &WorkspaceState, id: &str) -> Result<(), QueueError> {
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM deployment_plans WHERE id = ?1", [id])?;
                Ok(())
            })
            .map_err(QueueError::from)
    }

    pub fn agent_submission(
        state: &WorkspaceState,
        id: &str,
        keepass_password: Option<&str>,
    ) -> Result<AgentJobSetSubmission, QueueError> {
        state
            .with_connection(|connection| {
                let plan = connection
                    .query_row(
                        "SELECT p.plan_number, p.name, p.process_name, p.definition_yaml, c.definition_json
                         FROM deployment_plans p
                         JOIN deployment_processes c ON c.name = p.process_name COLLATE NOCASE
                         WHERE p.id = ?1",
                        [id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        },
                    )
                    .optional()?;
                let Some((plan_number, name, process_name, plan_definition, process_definition)) = plan else {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                };
                let mut statement = connection.prepare(
                    "SELECT id, sequence, name, driver, action, target, details, payload_json
                     FROM deployment_jobs WHERE plan_id = ?1 ORDER BY sequence",
                )?;
                let jobs = statement
                    .query_map([id], |row| {
                        Ok(AgentJobSubmission {
                            id: row.get(0)?,
                            sequence: row.get(1)?,
                            name: row.get(2)?,
                            driver: row.get(3)?,
                            action: row.get(4)?,
                            target: row.get(5)?,
                            details: row.get(6)?,
                            payload: row.get(7)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let credentials = resolve_agent_credentials(
                    connection,
                    &process_definition,
                    &plan_definition,
                    &jobs,
                    keepass_password,
                )?;
                Ok(AgentJobSetSubmission {
                    id: id.to_owned(),
                    plan_number,
                    name,
                    process_name,
                    plan_definition,
                    process_definition,
                    jobs,
                    credentials,
                })
            })
            .map_err(map_workspace_error)
    }

    pub fn control(
        state: &WorkspaceState,
        item_type: &str,
        id: &str,
        action: &str,
    ) -> Result<(), QueueError> {
        if !matches!(
            action,
            "start" | "stop" | "restart" | "complete" | "fail" | "approve"
        ) {
            return Err(QueueError::Invalid(format!(
                "Unknown Queue action '{action}'."
            )));
        }
        state
            .with_connection(|connection| {
                let transaction = connection.unchecked_transaction()?;
                match item_type {
                    "plan" => control_plan(&transaction, id, action).map_err(to_sql_error)?,
                    "job" => control_job(&transaction, id, action).map_err(to_sql_error)?,
                    _ => {
                        return Err(to_sql_error(QueueError::Invalid(format!(
                            "Unknown Queue item type '{item_type}'."
                        ))));
                    }
                }
                transaction.commit()
            })
            .map_err(map_workspace_error)
    }
}

fn resolve_agent_credentials(
    connection: &Connection,
    process_definition: &str,
    plan_definition: &str,
    jobs: &[AgentJobSubmission],
    keepass_password: Option<&str>,
) -> Result<Vec<AgentCredential>, rusqlite::Error> {
    let needs_cx = jobs
        .iter()
        .any(|job| job.driver == "cx" && job.action == "validate");
    let needs_jira = jobs
        .iter()
        .any(|job| job.driver == "jira" && job.action == "validate");
    let needs_olam = jobs
        .iter()
        .any(|job| job.driver == "olam" && job.action == "deploy");
    if !needs_cx && !needs_jira && !needs_olam {
        return Ok(Vec::new());
    }
    let Some(password) = keepass_password.filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let process: JsonValue = serde_json::from_str(process_definition)
        .map_err(|error| to_sql_error(QueueError::Json(error)))?;
    let mut accounts: Vec<String> = Vec::new();
    for (needed, driver) in [(needs_jira, "jira"), (needs_cx, "cx"), (needs_olam, "olam")] {
        if !needed {
            continue;
        }
        let pointer = format!("/details/{driver}/account");
        let account = process
            .pointer(&pointer)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                to_sql_error(QueueError::Invalid(format!(
                    "Process Config details.{driver}.account is required for {driver} validation."
                )))
            })?;
        if !accounts
            .iter()
            .any(|value| value.eq_ignore_ascii_case(account))
        {
            accounts.push(account.to_owned());
        }
    }
    if needs_olam {
        let plan: PlanInput = serde_yaml::from_str(plan_definition)
            .map_err(|error| to_sql_error(QueueError::Yaml(error)))?;
        let account = plan.inputs.olam.account.trim();
        if !accounts
            .iter()
            .any(|value| value.eq_ignore_ascii_case(account))
        {
            accounts.push(account.to_owned());
        }
    }
    let database_path: String = connection
        .query_row(
            "SELECT value FROM vault_settings WHERE key = 'keepass_database_path'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .filter(|value: &String| !value.trim().is_empty())
        .ok_or_else(|| {
            to_sql_error(QueueError::Invalid(
                "Choose a KeePass database in Vaults first.".into(),
            ))
        })?;
    let mut credentials = Vec::new();
    for account in accounts {
        let entry_path: String = connection
            .query_row(
                "SELECT entry_path FROM vault_entries WHERE name = ?1 COLLATE NOCASE",
                [&account],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                to_sql_error(QueueError::Invalid(format!(
                    "Vault account '{account}' does not exist."
                )))
            })?;
        let credential = read_keepass_entry(&database_path, &entry_path, password)
            .map_err(|message| to_sql_error(QueueError::Invalid(message)))?;
        credentials.push(AgentCredential {
            reference: account.into(),
            username: credential.username,
            password: credential.password,
        });
    }
    Ok(credentials)
}

fn control_plan(connection: &Connection, id: &str, action: &str) -> Result<(), QueueError> {
    if matches!(action, "complete" | "fail") {
        return Err(QueueError::Invalid(format!(
            "The '{action}' action is only valid for Queue jobs."
        )));
    }
    let changed = match action {
        "stop" => {
            connection.execute("UPDATE deployment_jobs SET status = 'stopped' WHERE plan_id = ?1 AND status = 'running'", [id])?;
            connection.execute("UPDATE deployment_plans SET status = 'stopped', updated_at = CURRENT_TIMESTAMP WHERE id = ?1", [id])?
        }
        "restart" => {
            connection.execute(
                "UPDATE deployment_jobs SET status = 'queued' WHERE plan_id = ?1",
                [id],
            )?;
            let changed = connection.execute("UPDATE deployment_plans SET status = 'running', updated_at = CURRENT_TIMESTAMP WHERE id = ?1", [id])?;
            start_next_job(connection, id)?;
            changed
        }
        _ => {
            let changed = connection.execute("UPDATE deployment_plans SET status = 'running', updated_at = CURRENT_TIMESTAMP WHERE id = ?1", [id])?;
            if !start_next_job(connection, id)? {
                connection.execute(
                    "UPDATE deployment_plans SET status = CASE
                        WHEN NOT EXISTS (SELECT 1 FROM deployment_jobs WHERE plan_id = ?1 AND status != 'completed') THEN 'completed'
                        ELSE 'failed'
                     END, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    [id],
                )?;
            }
            changed
        }
    };
    if changed == 0 {
        return Err(QueueError::Invalid("Queue Plan does not exist.".into()));
    }
    Ok(())
}

fn start_next_job(connection: &Connection, plan_id: &str) -> rusqlite::Result<bool> {
    let changed = connection.execute(
        "UPDATE deployment_jobs SET status = 'running'
         WHERE id = (
            SELECT id FROM deployment_jobs
            WHERE plan_id = ?1 AND status != 'completed'
            ORDER BY sequence LIMIT 1
         ) AND status IN ('queued', 'stopped')",
        [plan_id],
    )?;
    Ok(changed == 1)
}

fn control_job(connection: &Connection, id: &str, action: &str) -> Result<(), QueueError> {
    let (plan_id, sequence, current_status): (String, i64, String) = connection
        .query_row(
            "SELECT plan_id, sequence, status FROM deployment_jobs WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                QueueError::Invalid("Queue job does not exist.".into())
            }
            other => QueueError::Sqlite(other),
        })?;

    match action {
        "stop" => {
            require_job_status(&current_status, &["running"], "stop")?;
            connection.execute(
                "UPDATE deployment_jobs SET status = 'stopped' WHERE id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE deployment_plans SET status = 'stopped', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                [&plan_id],
            )?;
        }
        "complete" => {
            require_job_status(&current_status, &["running"], "complete")?;
            connection.execute(
                "UPDATE deployment_jobs SET status = 'completed' WHERE id = ?1",
                [id],
            )?;
            if start_next_job(connection, &plan_id)? {
                connection.execute(
                    "UPDATE deployment_plans SET status = 'running', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    [&plan_id],
                )?;
            } else {
                connection.execute(
                    "UPDATE deployment_plans SET status = CASE
                        WHEN NOT EXISTS (SELECT 1 FROM deployment_jobs WHERE plan_id = ?1 AND status != 'completed') THEN 'completed'
                        ELSE 'failed'
                     END, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    [&plan_id],
                )?;
            }
        }
        "fail" => {
            require_job_status(&current_status, &["running"], "fail")?;
            connection.execute(
                "UPDATE deployment_jobs SET status = 'failed' WHERE id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE deployment_plans SET status = 'failed', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                [&plan_id],
            )?;
        }
        "approve" => {
            require_job_status(&current_status, &["awaiting_approval"], "approve")?;
            connection.execute(
                "UPDATE deployment_jobs SET status = 'running' WHERE id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE deployment_plans SET status = 'running', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                [&plan_id],
            )?;
        }
        "start" | "restart" => {
            let allowed_statuses: &[&str] = if action == "start" {
                &["queued", "stopped"]
            } else {
                &["failed", "warning", "completed", "stopped"]
            };
            require_job_status(&current_status, allowed_statuses, action)?;
            let incomplete_predecessors: bool = connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM deployment_jobs
                    WHERE plan_id = ?1 AND sequence < ?2 AND status != 'completed'
                 )",
                params![plan_id, sequence],
                |row| row.get(0),
            )?;
            if incomplete_predecessors {
                return Err(QueueError::Invalid(
                    "This job can start only after every previous job has completed successfully."
                        .into(),
                ));
            }
            connection.execute(
                "UPDATE deployment_jobs SET status = 'queued' WHERE plan_id = ?1 AND sequence > ?2",
                params![plan_id, sequence],
            )?;
            connection.execute(
                "UPDATE deployment_jobs SET status = 'running' WHERE id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE deployment_plans SET status = 'running', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                [&plan_id],
            )?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn require_job_status(
    current_status: &str,
    allowed_statuses: &[&str],
    action: &str,
) -> Result<(), QueueError> {
    if allowed_statuses.contains(&current_status) {
        return Ok(());
    }
    Err(QueueError::Invalid(format!(
        "Cannot {action} a Queue job while its status is '{current_status}'."
    )))
}

fn validate_input(input: &PlanInput) -> Result<(), QueueError> {
    if input.plan_name.trim().is_empty() {
        return Err(QueueError::Invalid("plan_name is required.".into()));
    }
    if input.process.trim().is_empty() {
        return Err(QueueError::Invalid("process is required.".into()));
    }
    if input.inputs.olam.strategy.trim().is_empty() {
        return Err(QueueError::Invalid(
            "inputs.olam.strategy is required.".into(),
        ));
    }
    if input.inputs.olam.account.trim().is_empty() {
        return Err(QueueError::Invalid(
            "inputs.olam.account is required.".into(),
        ));
    }
    if input.inputs.olam.steps.is_empty() {
        return Err(QueueError::Invalid(
            "inputs.olam.steps must not be empty.".into(),
        ));
    }
    if input.inputs.olam.targets.hosts.is_empty() {
        return Err(QueueError::Invalid(
            "inputs.olam.targets.hosts must not be empty.".into(),
        ));
    }
    if input.inputs.olam.targets.roles.is_empty()
        || input
            .inputs
            .olam
            .targets
            .roles
            .iter()
            .any(|role| role.trim().is_empty())
    {
        return Err(QueueError::Invalid(
            "inputs.olam.targets.roles must contain at least one role.".into(),
        ));
    }
    if input
        .inputs
        .olam
        .targets
        .hosts
        .iter()
        .any(|host| host.trim().is_empty())
    {
        return Err(QueueError::Invalid(
            "Every OLAM target host must have a value.".into(),
        ));
    }
    if input
        .inputs
        .olam
        .steps
        .iter()
        .any(|step| step.name.trim().is_empty())
    {
        return Err(QueueError::Invalid("Every OLAM step needs a name.".into()));
    }
    if input
        .inputs
        .olam
        .steps
        .iter()
        .any(|step| !step.extra_vars.is_null() && !matches!(step.extra_vars, YamlValue::Mapping(_)))
    {
        return Err(QueueError::Invalid(
            "OLAM step extra_vars must be a key-value object.".into(),
        ));
    }
    if input
        .inputs
        .jira
        .as_ref()
        .is_some_and(|jira| jira.issue.trim().is_empty())
    {
        return Err(QueueError::Invalid("inputs.jira.issue is required.".into()));
    }
    if input
        .inputs
        .cx
        .as_ref()
        .is_some_and(|cx| cx.request.trim().is_empty())
    {
        return Err(QueueError::Invalid("inputs.cx.request is required.".into()));
    }
    Ok(())
}

fn validate_references(connection: &Connection, input: &PlanInput) -> Result<(), QueueError> {
    let process_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM deployment_processes WHERE name = ?1 COLLATE NOCASE)",
        [input.process.trim()],
        |row| row.get(0),
    )?;
    if !process_exists {
        return Err(QueueError::Invalid(format!(
            "Process Config '{}' does not exist.",
            input.process.trim()
        )));
    }
    let account_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM vault_entries WHERE name = ?1 COLLATE NOCASE)",
        [input.inputs.olam.account.trim()],
        |row| row.get(0),
    )?;
    if !account_exists {
        return Err(QueueError::Invalid(format!(
            "Vault account '{}' does not exist.",
            input.inputs.olam.account.trim()
        )));
    }
    Ok(())
}

fn load_process_payload_defaults(
    connection: &Connection,
    input: &PlanInput,
) -> Result<JsonMap<String, JsonValue>, QueueError> {
    let definition: String = connection.query_row(
        "SELECT definition_json FROM deployment_processes WHERE name = ?1 COLLATE NOCASE",
        [input.process.trim()],
        |row| row.get(0),
    )?;
    let process: JsonValue = serde_json::from_str(&definition)?;
    let olam = process
        .get("details")
        .and_then(|details| details.get("olam"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| QueueError::Invalid("Process Config details.olam is required.".into()))?;
    let mut defaults = olam
        .get("payload_defaults")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let environment = defaults
        .get("ENVIRONMENT")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            QueueError::Invalid(
                "Process Config details.olam.payload_defaults.ENVIRONMENT is required.".into(),
            )
        })?;
    let environment = environment.to_owned();
    defaults.insert("ENVIRONMENT".into(), JsonValue::String(environment));
    Ok(defaults)
}

fn load_process_olam_account(
    connection: &Connection,
    input: &PlanInput,
) -> Result<String, QueueError> {
    let definition: String = connection.query_row(
        "SELECT definition_json FROM deployment_processes WHERE name = ?1 COLLATE NOCASE",
        [input.process.trim()],
        |row| row.get(0),
    )?;
    let process: JsonValue = serde_json::from_str(&definition)?;
    process
        .pointer("/details/olam/account")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            QueueError::Invalid(
                "Process Config details.olam.account is required for OLAM API validation.".into(),
            )
        })
}

fn generate_jobs(
    input: &PlanInput,
    payload_defaults: &JsonMap<String, JsonValue>,
) -> Result<Vec<JobDefinition>, QueueError> {
    let mut jobs = Vec::new();
    if input.inputs.jira.is_some() {
        jobs.push(JobDefinition {
            name: "Validate Jira".into(),
            target: Some(
                input
                    .inputs
                    .jira
                    .as_ref()
                    .expect("Jira input")
                    .issue
                    .trim()
                    .into(),
            ),
            details: "Jira validation".into(),
            payload: None,
            driver: "jira",
            action: "validate",
        });
    }
    if input.inputs.cx.is_some() {
        jobs.push(JobDefinition {
            name: "Validate CX".into(),
            target: Some(
                input
                    .inputs
                    .cx
                    .as_ref()
                    .expect("CX input")
                    .request
                    .trim()
                    .into(),
            ),
            details: "CX validation".into(),
            payload: None,
            driver: "cx",
            action: "validate",
        });
    }
    for step in &input.inputs.olam.steps {
        for host in &input.inputs.olam.targets.hosts {
            jobs.push(JobDefinition {
                name: step.name.trim().to_owned(),
                target: Some(host.trim().to_owned()),
                details: "Validation pending".into(),
                payload: Some(build_olam_payload(
                    input,
                    Some(host),
                    Some(step),
                    payload_defaults,
                )?),
                driver: "olam",
                action: "deploy",
            });
        }
    }
    if input.inputs.jira.is_some() {
        jobs.push(JobDefinition {
            name: "Update Jira".into(),
            target: Some(
                input
                    .inputs
                    .jira
                    .as_ref()
                    .expect("Jira input")
                    .issue
                    .trim()
                    .into(),
            ),
            details: "Jira update".into(),
            payload: None,
            driver: "jira",
            action: "update",
        });
    }
    if input.inputs.cx.is_some() {
        jobs.push(JobDefinition {
            name: "Update CX".into(),
            target: Some(
                input
                    .inputs
                    .cx
                    .as_ref()
                    .expect("CX input")
                    .request
                    .trim()
                    .into(),
            ),
            details: "CX update".into(),
            payload: None,
            driver: "cx",
            action: "update",
        });
    }
    Ok(jobs)
}

fn build_olam_payload(
    input: &PlanInput,
    host: Option<&str>,
    step: Option<&PlanStep>,
    payload_defaults: &JsonMap<String, JsonValue>,
) -> Result<String, QueueError> {
    let mut payload = payload_defaults.clone();
    if let Some(step) = step
        && !step.extra_vars.is_null()
    {
        let extra_vars = serde_json::to_value(&step.extra_vars)?;
        let extra_vars = extra_vars.as_object().ok_or_else(|| {
            QueueError::Invalid("OLAM step extra_vars must be a key-value object.".into())
        })?;
        payload.extend(extra_vars.clone());
    }

    let hosts = host
        .map(|host| vec![JsonValue::String(host.trim().to_owned())])
        .unwrap_or_else(|| {
            input
                .inputs
                .olam
                .targets
                .hosts
                .iter()
                .map(|host| JsonValue::String(host.trim().to_owned()))
                .collect()
        });
    payload.insert("ansible_become_pass".into(), json!("$encrypted$"));
    payload.insert("ansible_limit".into(), JsonValue::Array(hosts));
    payload.insert("ansible_password".into(), json!("$encrypted$"));
    payload.insert("ansible_user".into(), json!("$encrypted$"));
    payload.insert(
        "playbook_name".into(),
        json!(input.inputs.olam.targets.roles[0].trim()),
    );
    payload.insert("STRATEGY".into(), json!(input.inputs.olam.strategy.trim()));
    if let Some(cx) = &input.inputs.cx {
        payload.insert("ticket_id".into(), json!(cx.request.trim()));
        payload.insert("ticket_system".into(), json!("CX"));
    } else if let Some(jira) = &input.inputs.jira {
        payload.insert("ticket_id".into(), json!(jira.issue.trim()));
        payload.insert("ticket_system".into(), json!("JIRA"));
    }
    serde_json::to_string_pretty(&JsonValue::Object(payload)).map_err(QueueError::from)
}

fn insert_jobs(
    connection: &Connection,
    plan_id: &str,
    jobs: Vec<JobDefinition>,
) -> rusqlite::Result<()> {
    for (index, job) in jobs.into_iter().enumerate() {
        connection.execute(
            "INSERT INTO deployment_jobs(id, plan_id, sequence, name, target, details, payload_json, driver, action)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                Uuid::new_v4().to_string(),
                plan_id,
                index as i64 + 1,
                job.name,
                job.target,
                job.details,
                job.payload,
                job.driver,
                job.action
            ],
        )?;
    }
    Ok(())
}

fn upgrade_legacy_queued_plans(connection: &Connection) -> rusqlite::Result<()> {
    let plans = {
        let mut statement = connection.prepare(
            "SELECT p.id, p.definition_yaml
             FROM deployment_plans p
             WHERE p.status IN ('queued', 'stopped', 'failed')
               AND (
                   EXISTS (
                       SELECT 1 FROM deployment_jobs j
                       WHERE j.plan_id = p.id
                         AND j.driver = 'olam' AND j.action = 'validate'
                   )
                   OR (
                       EXISTS (
                           SELECT 1 FROM deployment_jobs j
                           WHERE j.plan_id = p.id
                             AND (
                                 (j.target IS NULL AND (j.action = 'deploy' OR j.driver IN ('jira', 'cx')))
                                 OR (j.driver = 'olam' AND j.payload_json IS NULL)
                             )
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM deployment_jobs j
                           WHERE j.plan_id = p.id AND j.status != 'queued'
                       )
                   )
               )",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for (plan_id, definition) in plans {
        let Ok(input) = serde_yaml::from_str::<PlanInput>(&definition) else {
            continue;
        };
        if validate_input(&input).is_err() {
            continue;
        }
        let Ok(defaults) = load_process_payload_defaults(connection, &input) else {
            continue;
        };
        if load_process_olam_account(connection, &input).is_err() {
            continue;
        }
        let Ok(jobs) = generate_jobs(&input, &defaults) else {
            continue;
        };
        let transaction = connection.unchecked_transaction()?;
        transaction.execute("DELETE FROM deployment_jobs WHERE plan_id = ?1", [&plan_id])?;
        insert_jobs(&transaction, &plan_id, jobs)?;
        transaction.commit()?;
    }
    Ok(())
}

fn load_plans(connection: &Connection) -> rusqlite::Result<Vec<QueuePlan>> {
    let mut statement = connection.prepare(
        "SELECT id, plan_number, name, process_name, definition_yaml, status, created_at
         FROM deployment_plans ORDER BY created_at DESC, rowid DESC",
    )?;
    let plans = statement
        .query_map([], |row| {
            Ok(QueuePlan {
                id: row.get(0)?,
                plan_number: row.get(1)?,
                name: row.get(2)?,
                process_name: row.get(3)?,
                definition: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
                jobs: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    plans
        .into_iter()
        .map(|mut plan| {
            let mut jobs = connection.prepare(
                "SELECT id, sequence, name, target, details, payload_json, log_output, driver, action, status
                 FROM deployment_jobs WHERE plan_id = ?1 ORDER BY sequence",
            )?;
            plan.jobs = jobs
                .query_map([&plan.id], |row| {
                    Ok(QueueJob {
                        id: row.get(0)?,
                        sequence: row.get(1)?,
                        name: row.get(2)?,
                        target: row.get(3)?,
                        details: row.get(4)?,
                        payload: row.get(5)?,
                        log: row.get(6)?,
                        driver: row.get(7)?,
                        action: row.get(8)?,
                        status: row.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(plan)
        })
        .collect()
}

fn to_sql_error(error: QueueError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn map_workspace_error(error: WorkspaceError) -> QueueError {
    if let WorkspaceError::Storage(StorageError::Database(
        rusqlite::Error::ToSqlConversionFailure(source),
    )) = error
    {
        return match source.downcast::<QueueError>() {
            Ok(error) => *error,
            Err(source) => QueueError::Invalid(source.to_string()),
        };
    }
    QueueError::Workspace(error)
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("{0}")]
    Invalid(String),
    #[error("Invalid Plan YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Invalid deployment JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Encrypted Queue storage error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Agent(#[from] AgentIpcError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::modules::workspace::WorkspaceService;

    use super::*;

    #[test]
    fn creates_a_grouped_queue_from_optional_inputs_and_olam_steps() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("pharos-queue-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .expect("create workspace");
        state.with_connection(|connection| {
            connection.execute(
                "INSERT INTO deployment_processes(id, name, definition_json) VALUES ('standard-deploy', 'Standard Deploy', ?1)",
                [r#"{"details":{"olam":{"account":"olam_api_account","payload_defaults":{"ENVIRONMENT":"test","escalation_contact":"team"}}}}"#],
            )?;
            connection.execute(
                "INSERT INTO vault_entries(id, name, entry_path) VALUES ('account-1', 'deployment_account', 'Accounts/deployment')",
                [],
            )?;
            connection.execute(
                "INSERT INTO vault_entries(id, name, entry_path) VALUES ('account-2', 'olam_api_account', 'Accounts/olam-api')",
                [],
            )?;
            Ok(())
        }).expect("seed references");

        QueueService::create(
            &state,
            r#"
plan_name: Platform upgrade
process: Standard Deploy
inputs:
  jira:
    issue: APP-42
  cx:
    request: REQ-100
  olam:
    strategy: linear
    account: deployment_account
    targets:
      sites: [test]
      roles: [service]
      hosts: [host-1, host-2]
    steps:
      - name: Upgrade phase one
        extra_vars: { VERSION: "1" }
      - name: Upgrade phase two
        extra_vars: { VERSION: "2" }
"#
            .into(),
        )
        .expect("create plan");

        let snapshot = QueueService::snapshot(&state).expect("load queue");
        assert_eq!(snapshot.plans.len(), 1);
        assert_eq!(snapshot.plans[0].jobs.len(), 8);
        assert_eq!(snapshot.plans[0].jobs[0].name, "Validate Jira");
        assert_eq!(snapshot.plans[0].jobs[2].name, "Upgrade phase one");
        assert_eq!(snapshot.plans[0].jobs[2].target.as_deref(), Some("host-1"));
        assert_eq!(snapshot.plans[0].jobs[3].name, "Upgrade phase one");
        assert_eq!(snapshot.plans[0].jobs[3].target.as_deref(), Some("host-2"));
        assert_eq!(snapshot.plans[0].jobs[4].name, "Upgrade phase two");
        assert_eq!(snapshot.plans[0].jobs[4].target.as_deref(), Some("host-1"));
        assert_eq!(snapshot.plans[0].jobs[5].name, "Upgrade phase two");
        assert_eq!(snapshot.plans[0].jobs[5].target.as_deref(), Some("host-2"));
        assert_eq!(snapshot.plans[0].jobs[7].name, "Update CX");
        assert_eq!(snapshot.plans[0].jobs[0].target.as_deref(), Some("APP-42"));
        assert_eq!(snapshot.plans[0].jobs[7].target.as_deref(), Some("REQ-100"));
        let payload: JsonValue = serde_json::from_str(
            snapshot.plans[0].jobs[2]
                .payload
                .as_deref()
                .expect("OLAM payload"),
        )
        .expect("valid OLAM payload JSON");
        assert_eq!(payload["ansible_limit"], json!(["host-1"]));
        assert_eq!(payload["ansible_user"], "$encrypted$");
        assert_eq!(payload["ENVIRONMENT"], "test");
        assert_eq!(payload["escalation_contact"], "team");
        assert_eq!(payload["VERSION"], "1");
        assert_eq!(payload["playbook_name"], "service");
        assert_eq!(payload["STRATEGY"], "linear");
        assert_eq!(payload["ticket_id"], "REQ-100");
        assert_eq!(payload["ticket_system"], "CX");
        let submission = QueueService::agent_submission(&state, &snapshot.plans[0].id, None)
            .expect("build Agent job set submission");
        assert_eq!(submission.name, "Platform upgrade");
        assert_eq!(submission.jobs.len(), 8);
        assert_eq!(submission.jobs[2].target.as_deref(), Some("host-1"));
        assert!(submission.jobs[2].payload.is_some());

        let plan_id = snapshot.plans[0].id.clone();
        let updated_definition = snapshot.plans[0]
            .definition
            .replace("APP-42", "APP-43")
            .replace("REQ-100", "REQ-200");
        QueueService::update(&state, &plan_id, updated_definition).expect("update plan payload");
        let snapshot = QueueService::snapshot(&state).expect("load updated queue");
        assert_eq!(snapshot.plans[0].jobs[0].target.as_deref(), Some("APP-43"));
        assert_eq!(snapshot.plans[0].jobs[7].target.as_deref(), Some("REQ-200"));

        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE deployment_jobs SET target = NULL WHERE action = 'deploy'",
                    [],
                )?;
                Ok(())
            })
            .expect("simulate legacy queued jobs");
        let snapshot = QueueService::snapshot(&state).expect("upgrade legacy queue jobs");
        assert_eq!(snapshot.plans[0].jobs.len(), 8);
        assert_eq!(snapshot.plans[0].jobs[2].target.as_deref(), Some("host-1"));
        assert_eq!(snapshot.plans[0].jobs[5].target.as_deref(), Some("host-2"));

        let first_job_id = snapshot.plans[0].jobs[0].id.clone();
        let second_job_id = snapshot.plans[0].jobs[1].id.clone();
        let third_job_id = snapshot.plans[0].jobs[2].id.clone();

        let error = QueueService::control(&state, "job", &third_job_id, "start")
            .expect_err("later jobs must wait for successful predecessors");
        assert!(error.to_string().contains("previous job"));

        QueueService::control(&state, "plan", &plan_id, "start").expect("start plan");
        let snapshot = QueueService::snapshot(&state).expect("load running queue");
        assert_eq!(snapshot.plans[0].status, "running");
        assert_eq!(snapshot.plans[0].jobs[0].status, "running");
        assert_eq!(snapshot.plans[0].jobs[1].status, "queued");

        QueueService::control(&state, "job", &first_job_id, "complete")
            .expect("complete first job");
        let snapshot = QueueService::snapshot(&state).expect("load advanced queue");
        assert_eq!(snapshot.plans[0].jobs[0].status, "completed");
        assert_eq!(snapshot.plans[0].jobs[1].status, "running");

        QueueService::control(&state, "job", &second_job_id, "stop").expect("stop second job");
        QueueService::control(&state, "plan", &plan_id, "start").expect("resume plan");
        let snapshot = QueueService::snapshot(&state).expect("load resumed queue");
        assert_eq!(snapshot.plans[0].jobs[0].status, "completed");
        assert_eq!(snapshot.plans[0].jobs[1].status, "running");

        QueueService::control(&state, "plan", &plan_id, "restart").expect("restart plan");
        let snapshot = QueueService::snapshot(&state).expect("load restarted queue");
        assert_eq!(snapshot.plans[0].jobs[0].status, "running");
        assert!(
            snapshot.plans[0].jobs[1..]
                .iter()
                .all(|job| job.status == "queued")
        );

        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
