use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use serde_yaml::Value as YamlValue;
use thiserror::Error;
use uuid::Uuid;

use crate::agent_ipc::{
    AgentCredential, AgentIpcError, AgentJobSetStatus, AgentJobSetSubmission, AgentJobStatus,
    AgentJobSubmission,
};
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
    #[serde(default, alias = "ansible", alias = "container")]
    olam: OlamInput,
}

#[derive(Debug, Deserialize)]
struct JiraInput {
    issue: String,
}

#[derive(Debug, Deserialize)]
struct CxInput {
    request: String,
    #[serde(default)]
    requester: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OlamInput {
    strategy: String,
    account: String,
    targets: OlamTargets,
    steps: Vec<PlanStep>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OlamTargets {
    #[serde(default, rename = "sites")]
    _sites: Vec<String>,
    roles: Vec<String>,
    hosts: Vec<String>,
    inventory_groups: Vec<String>,
    job_mode: Option<OlamJobMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OlamJobMode { PerHost, Together }

#[derive(Debug, Deserialize)]
struct PlanStep {
    name: String,
    #[serde(default, rename = "extra_vars")]
    extra_vars: YamlValue,
}

#[derive(Debug)]
struct JobDefinition {
    name: String,
    target: Option<String>,
    details: String,
    payload: Option<String>,
    driver: String,
    action: String,
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
                disabled: job.status == "disabled",
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
        Self::synchronize_job_sets(state, &job_sets)
    }

    pub fn synchronize_job_sets(
        state: &WorkspaceState,
        job_sets: &[AgentJobSetStatus],
    ) -> Result<(), QueueError> {
        state.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            for set in job_sets {
                let Some(first) = set.jobs.first() else { continue; };
                let current_run: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM deployment_jobs WHERE plan_id = ?1 AND id = ?2)",
                    params![set.id, first.id], |row| row.get(0),
                )?;
                if !current_run { continue; }
                transaction.execute(
                    "UPDATE deployment_plans SET status = ?2, approve_all = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![set.id, set.status, set.approve_all],
                )?;
                for job in &set.jobs {
                    transaction.execute(
                        "UPDATE deployment_jobs SET status = ?2, log_output = ?3, details = ?4
                         WHERE id = ?1 AND plan_id = ?5",
                        params![job.id, job.status, job.log, agent_job_details(job), set.id],
                    )?;
                }
            }
            transaction.commit()
        }).map_err(QueueError::from)
    }

    pub fn set_approve_all(state: &WorkspaceState, id: &str, enabled: bool) -> Result<(), QueueError> {
        state.with_connection(|connection| {
            if connection.execute("UPDATE deployment_plans SET approve_all = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1", params![id, enabled])? == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        }).map_err(map_workspace_error)
    }

    pub fn snapshot(state: &WorkspaceState) -> Result<QueueSnapshot, QueueError> {
        state
            .with_connection(|connection| {
                compact_queued_endpoint_checks(connection)?;
                Ok(QueueSnapshot {
                    plans: load_plans(connection)?,
                })
            })
            .map_err(map_workspace_error)
    }

    pub fn create(state: &WorkspaceState, definition: String) -> Result<String, QueueError> {
        let input: PlanInput = serde_yaml::from_str(&definition)?;
        validate_input(&input)?;
        state
            .with_connection(|connection| {
                let process_definition = load_process_definition(connection, &input)?;
                let process: JsonValue = serde_json::from_str(&process_definition)
                    .map_err(|error| to_sql_error(QueueError::Json(error)))?;
                let jobs = generate_jobs(&input, &process).map_err(to_sql_error)?;
                validate_references(connection, &input, &jobs).map_err(to_sql_error)?;
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
                    "INSERT INTO deployment_plans(id, plan_number, name, process_name, definition_yaml, process_definition_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        plan_id,
                        plan_number,
                        input.plan_name.trim(),
                        input.process.trim(),
                        definition,
                        process_definition
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
                transaction.commit()?;
                Ok(plan_id)
            })
            .map_err(map_workspace_error)
    }

    pub fn update(state: &WorkspaceState, id: &str, definition: String) -> Result<(), QueueError> {
        let input: PlanInput = serde_yaml::from_str(&definition)?;
        validate_input(&input)?;
        state
            .with_connection(|connection| {
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
                if matches!(status.as_str(), "running" | "awaiting_approval") {
                    return Err(to_sql_error(QueueError::Invalid(
                        "A running Queue Plan cannot be edited.".into(),
                    )));
                }

                let process_definition = load_process_definition(connection, &input)?;
                let process: JsonValue = serde_json::from_str(&process_definition)
                    .map_err(|error| to_sql_error(QueueError::Json(error)))?;
                let jobs = generate_jobs(&input, &process).map_err(to_sql_error)?;
                validate_references(connection, &input, &jobs).map_err(to_sql_error)?;
                let transaction = connection.unchecked_transaction()?;
                transaction.execute(
                    "UPDATE deployment_plans
                     SET name = ?2, process_name = ?3, definition_yaml = ?4,
                         status = 'queued', process_definition_json = ?5, updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![id, input.plan_name.trim(), input.process.trim(), definition, process_definition],
                )?;
                let disabled = {
                    let mut query = transaction.prepare("SELECT name, driver, action, target, payload_json FROM deployment_jobs WHERE plan_id = ?1 AND status = 'disabled'")?;
                    let rows = query.query_map([id], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,Option<String>>(3)?, row.get::<_,Option<String>>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                };
                transaction.execute("DELETE FROM deployment_jobs WHERE plan_id = ?1", [id])?;
                insert_jobs(&transaction, id, jobs)?;
                for (name, driver, action, target, payload) in disabled {
                    transaction.execute("UPDATE deployment_jobs SET status = 'disabled' WHERE plan_id = ?1 AND name = ?2 AND driver = ?3 AND action = ?4 AND target IS ?5 AND payload_json IS ?6",
                        params![id, name, driver, action, target, payload])?;
                }
                transaction.commit()
            })
            .map_err(map_workspace_error)
    }

    pub fn prepare_plan_start(
        state: &WorkspaceState,
        id: &str,
        restart: bool,
    ) -> Result<(), QueueError> {
        state
            .with_connection(compact_queued_endpoint_checks)
            .map_err(QueueError::from)?;
        let (status, definition, legacy) = state.with_connection(|connection| {
            connection.query_row(
                "SELECT status, definition_yaml, process_definition_json IS NULL FROM deployment_plans WHERE id = ?1",
                [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?)),
            )
        }).map_err(QueueError::from)?;
        if matches!(status.as_str(), "running" | "awaiting_approval") {
            return Err(QueueError::Invalid(
                "Stop the Plan before starting a new run.".into(),
            ));
        }
        if restart || legacy || status == "stopped" {
            Self::update(state, id, definition)?;
        }
        Ok(())
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
                        "SELECT p.plan_number, p.name, p.process_name, p.definition_yaml, COALESCE(p.process_definition_json, c.definition_json), p.approve_all
                         FROM deployment_plans p
                         LEFT JOIN deployment_processes c ON c.name = p.process_name COLLATE NOCASE
                         WHERE p.id = ?1",
                        [id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, bool>(5)?,
                            ))
                        },
                    )
                    .optional()?;
                let Some((plan_number, name, process_name, plan_definition, process_definition, approve_all)) = plan else {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                };
                let process_definition = process_with_plan_requester(&process_definition, &plan_definition)?;
                let mut statement = connection.prepare(
                    "SELECT id, sequence, name, driver, action, target, details, payload_json, status = 'disabled'
                     FROM deployment_jobs WHERE plan_id = ?1 ORDER BY sequence",
                )?;
                let jobs = statement
                    .query_map([id], |row| {
                        Ok(AgentJobSubmission {
                            disabled: row.get(8)?,
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
                    approve_all,
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
            "start" | "stop" | "restart" | "complete" | "fail" | "approve" | "disable" | "enable"
        ) {
            return Err(QueueError::Invalid(format!(
                "Unknown Queue action '{action}'."
            )));
        }
        if item_type != "job" && matches!(action, "disable" | "enable") {
            return Err(QueueError::Invalid(
                "Disable and enable apply only to individual jobs.".into(),
            ));
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

fn agent_job_details(job: &AgentJobStatus) -> String {
    if job.status == "disabled" {
        return "Disabled — skipped during execution".into();
    }
    if !matches!(job.driver.as_str(), "olam" | "container") {
        return job.details.clone();
    }
    let phase = if job.action == "validate" && job.payload.is_none() {
        "Endpoint validation"
    } else if job.phase.as_deref() == Some("deployment") {
        "Deployment"
    } else {
        "Validation"
    };
    let description = match job.status.as_str() {
        "queued" => format!("{phase} pending"),
        "awaiting_approval" => {
            if job.validation_job_id.is_some()
                || logged_olam_job_id(&job.log, "validation").is_some()
            {
                "Validation passed - awaiting approval".into()
            } else {
                "Deployment awaiting approval".into()
            }
        }
        "denied" => "Deployment denied".into(),
        "running" => format!("{phase} running"),
        "completed" => format!("{phase} completed"),
        "failed" => format!("{phase} failed"),
        "warning" => format!("{phase} warning"),
        "stopped" => format!("{phase} stopped"),
        _ => job.details.clone(),
    };
    if job.status == "queued" {
        return description;
    }
    let validation =
        job.status == "awaiting_approval" || job.status == "denied" || phase == "Validation";
    let external_id = if validation {
        job.validation_job_id
    } else {
        job.deployment_job_id
    }
    .filter(|id| *id > 0)
    .or_else(|| {
        logged_olam_job_id(
            &job.log,
            if validation {
                "validation"
            } else {
                "deployment"
            },
        )
    });
    match external_id {
        Some(id) => format!("{description} · Job #{id}"),
        None => description,
    }
}

fn logged_olam_job_id(log: &str, phase: &str) -> Option<i64> {
    let prefix = format!("OLAM {phase} ");
    let boundary = format!("Starting OLAM {phase} for ");
    let log = log.rsplit(&boundary).next().unwrap_or(log);
    log.lines().rev().find_map(|line| {
        if !line.starts_with(&prefix) {
            return None;
        }
        let (_, suffix) = line.split_once(" as Ansible job ")?;
        let number: String = suffix.chars().take_while(char::is_ascii_digit).collect();
        number.parse::<i64>().ok().filter(|id| *id > 0)
    })
}

fn resolve_agent_credentials(
    connection: &Connection,
    process_definition: &str,
    plan_definition: &str,
    jobs: &[AgentJobSubmission],
    keepass_password: Option<&str>,
) -> Result<Vec<AgentCredential>, rusqlite::Error> {
    let needs_cx = jobs.iter().any(|job| job.driver == "cx");
    let needs_jira = jobs.iter().any(|job| job.driver == "jira");
    let needs_olam = jobs.iter().any(|job| job.driver == "olam");
    let needs_container = jobs.iter().any(|job| job.driver == "container");
    if !needs_cx && !needs_jira && !needs_olam && !needs_container {
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
    if jobs
        .iter()
        .any(|job| matches!(job.driver.as_str(), "olam" | "container") && job.action != "validate")
    {
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
                "UPDATE deployment_jobs SET status = 'queued' WHERE plan_id = ?1 AND status != 'disabled'",
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
                        WHEN NOT EXISTS (SELECT 1 FROM deployment_jobs WHERE plan_id = ?1 AND status NOT IN ('completed', 'disabled')) THEN 'completed'
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
            WHERE plan_id = ?1 AND status NOT IN ('completed', 'disabled')
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
        "disable" | "enable" => {
            require_job_status(
                &current_status,
                if action == "disable" {
                    &["queued", "stopped"]
                } else {
                    &["disabled"]
                },
                action,
            )?;
            let active: bool = connection.query_row("SELECT status IN ('running', 'awaiting_approval') FROM deployment_plans WHERE id = ?1", [&plan_id], |row| row.get(0))?;
            if active {
                return Err(QueueError::Invalid(
                    "The running Agent must update this job. Check the Agent connection.".into(),
                ));
            }
            connection.execute(
                "UPDATE deployment_jobs SET status = ?2 WHERE id = ?1",
                params![
                    id,
                    if action == "disable" {
                        "disabled"
                    } else {
                        "queued"
                    }
                ],
            )?;
        }
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
                        WHEN NOT EXISTS (SELECT 1 FROM deployment_jobs WHERE plan_id = ?1 AND status NOT IN ('completed', 'disabled')) THEN 'completed'
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
                    WHERE plan_id = ?1 AND sequence < ?2 AND status NOT IN ('completed', 'disabled')
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
                "UPDATE deployment_jobs SET status = 'queued' WHERE plan_id = ?1 AND sequence > ?2 AND status != 'disabled'",
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
    Ok(())
}

fn validate_olam_input(input: &PlanInput) -> Result<(), QueueError> {
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
    let targets = &input.inputs.olam.targets;
    if targets.hosts.is_empty() && targets.inventory_groups.is_empty() {
        return Err(QueueError::Invalid(
            "Set inputs.olam.targets.hosts or inventory_groups.".into(),
        ));
    }
    if !targets.hosts.is_empty() && !targets.inventory_groups.is_empty() {
        return Err(QueueError::Invalid(
            "Use either inputs.olam.targets.hosts or inventory_groups, not both.".into(),
        ));
    }
    if targets.job_mode == Some(OlamJobMode::PerHost) && !targets.inventory_groups.is_empty() {
        return Err(QueueError::Invalid("Per-host jobs require hosts instead of inventory_groups. Choose hosts in Planner Builder or set targets.hosts explicitly.".into()));
    }
    if targets
        .inventory_groups
        .iter()
        .any(|group| group.trim().is_empty())
    {
        return Err(QueueError::Invalid(
            "Every OLAM inventory group must have a value.".into(),
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
    Ok(())
}

fn validate_references(
    connection: &Connection,
    input: &PlanInput,
    jobs: &[JobDefinition],
) -> Result<(), QueueError> {
    if jobs
        .iter()
        .any(|job| matches!(job.driver.as_str(), "olam" | "container") && job.action != "validate")
    {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM vault_entries WHERE name = ?1 COLLATE NOCASE)",
            [input.inputs.olam.account.trim()],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(QueueError::Invalid(format!(
                "Vault account '{}' does not exist.",
                input.inputs.olam.account.trim()
            )));
        }
    }
    Ok(())
}

fn effective_cx_requester<'a>(
    input: &'a PlanInput,
    step: &'a JsonValue,
    details: &'a JsonValue,
) -> Option<&'a str> {
    input
        .inputs
        .cx
        .as_ref()
        .and_then(|cx| cx.requester.as_deref())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            step.get("requester")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            details
                .get("requester")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
}

// Freeze the effective requester for each update step in the submitted snapshot.
// A common legacy default also lets an already-running older Agent execute a
// single-requester process. For differing identities, omit that default so an
// older Agent fails before writing instead of choosing the wrong requester.
fn process_with_plan_requester(process: &str, plan: &str) -> rusqlite::Result<String> {
    let input: PlanInput = serde_yaml::from_str(plan)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let mut process: JsonValue = serde_json::from_str(process)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let details = process
        .pointer("/details/cx")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let mut requesters = std::collections::BTreeSet::new();
    let mut has_updates = false;
    if let Some(steps) = process.get_mut("steps").and_then(JsonValue::as_array_mut) {
        for step in steps
            .iter_mut()
            .filter(|step| step["driver"] == "cx" && step["action"] == "update")
        {
            has_updates = true;
            if let Some(requester) =
                effective_cx_requester(&input, step, &details).map(str::to_owned)
            {
                step["requester"] = JsonValue::String(requester.clone());
                requesters.insert(requester);
            }
        }
    }
    if let Some(cx) = process
        .pointer_mut("/details/cx")
        .and_then(JsonValue::as_object_mut)
    {
        if has_updates && requesters.len() > 1 {
            cx.remove("requester");
        } else if let Some(requester) = requesters.into_iter().next().or_else(|| {
            effective_cx_requester(&input, &JsonValue::Null, &details).map(str::to_owned)
        }) {
            cx.insert("requester".into(), JsonValue::String(requester));
        }
    }
    serde_json::to_string(&process)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn load_process_definition(connection: &Connection, input: &PlanInput) -> rusqlite::Result<String> {
    connection
        .query_row(
            "SELECT definition_json FROM deployment_processes WHERE name = ?1 COLLATE NOCASE",
            [input.process.trim()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            to_sql_error(QueueError::Invalid(format!(
                "Process Config '{}' does not exist.",
                input.process.trim()
            )))
        })
}

fn generate_jobs(input: &PlanInput, process: &JsonValue) -> Result<Vec<JobDefinition>, QueueError> {
    let steps = process
        .get("steps")
        .and_then(JsonValue::as_array)
        .filter(|steps| !steps.is_empty())
        .ok_or_else(|| {
            QueueError::Invalid("Process steps must contain at least one step.".into())
        })?;
    let catalog: JsonValue = serde_json::from_str(include_str!("drivers/catalog.json"))?;
    let mut jobs = Vec::new();
    for step in steps {
        let field = |key: &str| {
            step.get(key)
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| QueueError::Invalid(format!("Every Process step requires {key}.")))
        };
        let name = field("name")?;
        let driver = field("driver")?;
        let action = field("action")?;
        if catalog
            .get(driver)
            .and_then(|driver| driver.get("actions"))
            .and_then(|actions| actions.get(action))
            .is_none()
        {
            return Err(QueueError::Invalid(format!(
                "Unsupported Process action {driver}/{action}."
            )));
        }
        let details = process
            .get("details")
            .and_then(|details| details.get(driver))
            .ok_or_else(|| {
                QueueError::Invalid(format!(
                    "Process step references missing details '{driver}'."
                ))
            })?;
        match driver {
            "jira" | "cx" => {
                let target = if driver == "jira" {
                    input.inputs.jira.as_ref().map(|input| input.issue.trim())
                } else {
                    input.inputs.cx.as_ref().map(|input| input.request.trim())
                }
                .filter(|value| !value.is_empty());
                // Ticket integrations are optional per Plan. Without a ticket,
                // neither validation nor report jobs exist for that driver.
                let Some(target) = target else { continue; };
                if driver == "cx" && action == "update" {
                    if step
                        .get("requester")
                        .is_some_and(|value| !value.is_string())
                    {
                        return Err(QueueError::Invalid(
                            "CX update step requester must be text.".into(),
                        ));
                    }
                    for field in ["url", "account"] {
                        if details
                            .get(field)
                            .and_then(JsonValue::as_str)
                            .is_none_or(|v| v.trim().is_empty())
                        {
                            return Err(QueueError::Invalid(format!(
                                "CX update requires details.cx.{field}."
                            )));
                        }
                    }
                }
                if driver == "cx"
                    && action == "update"
                    && effective_cx_requester(input, step, details).is_none()
                {
                    return Err(QueueError::Invalid(
                        "CX update requires inputs.cx.requester, a requester on its Process step, or details.cx.requester."
                            .into(),
                    ));
                }
                let report = if action == "update" {
                    let report = step.get("report").ok_or_else(|| {
                        QueueError::Invalid(
                            "Update requires an explicit report in the Process step.".into(),
                        )
                    })?;
                    (if driver == "cx" {
                        crate::agent_ipc::validate_cx_update_report
                    } else {
                        crate::agent_ipc::validate_jira_update_report
                    })(report)
                    .map_err(QueueError::Invalid)?;
                    Some(report.to_string())
                } else {
                    None
                };
                jobs.push(JobDefinition {
                    name: name.into(),
                    target: Some(target.into()),
                    details: format!("{} {action}", if driver == "jira" { "Jira" } else { "CX" }),
                    payload: report,
                    driver: driver.into(),
                    action: action.into(),
                });
            }
            "olam" if action == "validate" => {
                let url = details
                    .get("url")
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        QueueError::Invalid("Process details.olam.url is required.".into())
                    })?;
                jobs.push(JobDefinition {
                    name: name.into(),
                    target: Some(url.into()),
                    details: "Endpoint validation pending".into(),
                    payload: None,
                    driver: driver.into(),
                    action: action.into(),
                });
            }
            "olam" | "container" => {
                let execution_action = if action == "deploy" {
                    super::service::olam_deploy_action(step)
                        .map_err(|error| QueueError::Invalid(error.to_string()))?
                } else {
                    action
                };
                validate_olam_input(input)?;
                if driver == "container" && input.inputs.olam.targets._sites.is_empty() {
                    return Err(QueueError::Invalid("Container deployment requires at least one site in Planner.".into()));
                }
                let defaults = details
                    .get("payload_defaults")
                    .and_then(JsonValue::as_object)
                    .ok_or_else(|| {
                        QueueError::Invalid(
                            "Process details.olam.payload_defaults is required.".into(),
                        )
                    })?;
                if defaults
                    .get("ENVIRONMENT")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(QueueError::Invalid(
                        "Process details.olam.payload_defaults.ENVIRONMENT is required.".into(),
                    ));
                }
                if driver == "olam" && details
                    .get("account")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(QueueError::Invalid(
                        "Process details.olam.account is required.".into(),
                    ));
                }
                let targets = &input.inputs.olam.targets;
                let limits: Vec<String> = if targets.inventory_groups.is_empty() { &targets.hosts } else { &targets.inventory_groups }
                    .iter().map(|limit| limit.trim().to_owned()).collect();
                // Missing mode preserves existing Plans: hosts run separately,
                // while inventory_groups are sent together.
                let together = targets.job_mode == Some(OlamJobMode::Together)
                    || (targets.job_mode.is_none() && !targets.inventory_groups.is_empty());
                let target_sets: Vec<Vec<String>> = if together { vec![limits] } else { limits.into_iter().map(|host| vec![host]).collect() };
                for plan_step in &input.inputs.olam.steps {
                    for role in &input.inputs.olam.targets.roles {
                        for limits in &target_sets {
                            let mut job_name = format!("{name} · {}", plan_step.name.trim());
                            if input.inputs.olam.targets.roles.len() > 1 {
                                job_name.push_str(&format!(" · {}", role.trim()));
                            }
                            jobs.push(JobDefinition {
                                name: job_name,
                                target: Some(limits.join(", ")),
                                details: format!(
                                    "{} pending",
                                    if execution_action == "deploy" {
                                        "Deployment"
                                    } else {
                                        "Validation"
                                    }
                                ),
                                payload: Some({
                                    let mut payload: JsonValue = serde_json::from_str(&build_olam_payload(input, limits, Some(plan_step), role, defaults)?)?;
                                    if driver == "container" {
                                        payload["_container_sites"] = json!(input.inputs.olam.targets._sites);
                                    }
                                    serde_json::to_string_pretty(&payload)?
                                }),
                                driver: driver.into(),
                                action: execution_action.into(),
                            });
                        }
                    }
                }
            }
            _ => {
                return Err(QueueError::Invalid(format!(
                    "Unsupported driver '{driver}'."
                )));
            }
        }
    }
    Ok(jobs)
}

fn build_olam_payload(
    input: &PlanInput,
    limits: &[String],
    step: Option<&PlanStep>,
    role: &str,
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

    payload.insert("ansible_become_pass".into(), json!("$encrypted$"));
    payload.insert("ansible_limit".into(), json!(limits));
    payload.insert("ansible_password".into(), json!("$encrypted$"));
    payload.insert("ansible_user".into(), json!("$encrypted$"));
    payload.insert("playbook_name".into(), json!(role.trim()));
    payload.insert("STRATEGY".into(), json!(input.inputs.olam.strategy.trim()));
    if let Some(cx) = input.inputs.cx.as_ref().filter(|cx| !cx.request.trim().is_empty()) {
        payload.insert("ticket_id".into(), json!(cx.request.trim()));
        payload.insert("ticket_system".into(), json!("CX"));
    } else if let Some(jira) = input.inputs.jira.as_ref().filter(|jira| !jira.issue.trim().is_empty()) {
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

// Correct only never-started queues created by 0.1.7. Use their frozen
// configuration, not the current Process Config; preserve all executed history.
fn compact_queued_endpoint_checks(connection: &Connection) -> rusqlite::Result<()> {
    let plans = {
        let mut statement = connection.prepare(
            "SELECT p.id, p.definition_yaml, p.process_definition_json FROM deployment_plans p
             WHERE p.status='queued' AND p.process_definition_json IS NOT NULL
               AND EXISTS(SELECT 1 FROM deployment_jobs j WHERE j.plan_id=p.id AND j.driver='olam' AND j.action='validate' AND j.payload_json IS NOT NULL)
               AND NOT EXISTS(SELECT 1 FROM deployment_jobs j WHERE j.plan_id=p.id AND j.status!='queued')",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, definition, process) in plans {
        let Ok(input) = serde_yaml::from_str::<PlanInput>(&definition) else {
            continue;
        };
        let Ok(process) = serde_json::from_str::<JsonValue>(&process) else {
            continue;
        };
        let Ok(jobs) = generate_jobs(&input, &process) else {
            continue;
        };
        let transaction = connection.unchecked_transaction()?;
        transaction.execute("DELETE FROM deployment_jobs WHERE plan_id=?1", [&id])?;
        insert_jobs(&transaction, &id, jobs)?;
        transaction.commit()?;
    }
    Ok(())
}

fn load_plans(connection: &Connection) -> rusqlite::Result<Vec<QueuePlan>> {
    let mut statement = connection.prepare(
        "SELECT id, plan_number, name, process_name, definition_yaml, status, created_at, approve_all
         FROM deployment_plans ORDER BY created_at DESC, rowid DESC",
    )?;
    let plans = statement
        .query_map([], |row| {
            Ok(QueuePlan {
                approve_all: row.get(7)?,
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

    fn plan_input() -> PlanInput {
        serde_yaml::from_str(
            r#"
plan_name: Upgrade
process: Deploy
inputs:
  jira: {issue: APP-1}
  cx: {request: REQ-1}
  olam:
    strategy: linear
    account: deploy
    targets:
      roles: [service, exporter]
      hosts: [host-1, host-2]
    steps:
      - name: Version one
        extra_vars: {VERSION: "1"}
      - name: Version two
        extra_vars: {VERSION: "2"}
"#,
        )
        .unwrap()
    }

    fn process_config(steps: JsonValue) -> JsonValue {
        json!({"name":"Deploy", "steps": steps, "details": {
            "jira": {"account":"jira"}, "cx": {"url":"https://cx.test/requests", "account":"cx", "requester":"operator@example.test"},
            "olam": {"url":"https://olam.test", "account":"api", "payload_defaults":{"ENVIRONMENT":"test"}}
        }})
    }

    #[test]
    fn container_jobs_reuse_planner_grouping_and_explicit_workflow() {
        let mut input = plan_input();
        input.inputs.olam.targets.roles = vec!["apache_kafka".into()];
        input.inputs.olam.targets._sites = vec!["phx".into()];
        let config = json!({"name":"Devcorp - Test", "details":{"container":{
            "image":"registry.test/image:chosen", "deployment_playbook":"playbooks/deploy.yml",
            "payload_defaults":{"ENVIRONMENT":"devcorp"}}},
            "steps":[{"name":"Deploy Ansible", "driver":"container", "action":"deploy", "workflow":["validate","approve","deploy"]}]});
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(),4);
        for job in &jobs { assert_eq!(job.driver,"container"); assert_eq!(job.action,"validate_deploy");
            let payload:JsonValue=serde_json::from_str(job.payload.as_ref().unwrap()).unwrap();
            assert_eq!(payload["_container_sites"],json!(["phx"]));
            assert_eq!(payload["ansible_password"],"$encrypted$");
        }
        input.inputs.olam.targets.job_mode=Some(OlamJobMode::Together);
        let jobs=generate_jobs(&input,&config).unwrap(); assert_eq!(jobs.len(),2);
        let payload:JsonValue=serde_json::from_str(jobs[0].payload.as_ref().unwrap()).unwrap();
        assert_eq!(payload["ansible_limit"],json!(["host-1","host-2"]));
        let yaml="plan_name: Test\nprocess: Devcorp - Test\ninputs:\n  ansible:\n    strategy: linear\n    account: deploy\n    targets: {sites: [phx], roles: [apache_kafka], hosts: [host-1]}\n    steps: [{name: Test}]\n";
        let input:PlanInput=serde_yaml::from_str(yaml).unwrap();
        assert_eq!(generate_jobs(&input,&config).unwrap().len(),1);
        let mut invalid=config.clone();invalid["steps"][0].as_object_mut().unwrap().remove("workflow");
        assert!(generate_jobs(&input,&invalid).is_err());
    }

    #[test]
    #[ignore = "Explicit frontend YAML round-trip diagnostic"]
    fn planner_builder_yaml_round_trip() {
        let path = std::env::var("GATETREE_OPS_PLANNER_TEST_FILE").unwrap();
        let definition = fs::read_to_string(path).unwrap();
        let input: PlanInput = serde_yaml::from_str(&definition).unwrap();
        validate_input(&input).unwrap();
        validate_olam_input(&input).unwrap();
        assert_eq!(input.inputs.cx.as_ref().unwrap().request, "000001-000002");
        let config = process_config(json!([{"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["validate", "approve", "deploy"]}]));
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs[0].name.ends_with("Phase two"));
        assert!(jobs[1].name.ends_with("Phase one"));
        let first: JsonValue = serde_json::from_str(jobs[0].payload.as_ref().unwrap()).unwrap();
        let second: JsonValue = serde_json::from_str(jobs[1].payload.as_ref().unwrap()).unwrap();
        assert_eq!(first["ansible_limit"], json!(["env_a", "env_b"]));
        assert_eq!(first["COUNT"], "2");
        assert_eq!(second["VERSION"], "3.9.2");
    }

    #[test]
    fn generates_only_explicit_steps_in_exact_process_and_target_order() {
        let input = plan_input();
        let config = process_config(json!([
            {"name":"Check CX", "driver":"cx", "action":"validate"},
            {"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["approve","deploy"]},
            {"name":"Check Jira", "driver":"jira", "action":"validate"},
            {"name":"Validate", "driver":"olam", "action":"validate"}
        ]));
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 11);
        assert_eq!(
            (&*jobs[0].driver, &*jobs[0].action, &*jobs[0].name),
            ("cx", "validate", "Check CX")
        );
        assert!(
            jobs[1..9]
                .iter()
                .all(|job| job.driver == "olam" && job.action == "deploy")
        );
        assert_eq!(jobs[9].driver, "jira");
        assert!(
            jobs[10..]
                .iter()
                .all(|job| job.driver == "olam" && job.action == "validate")
        );
        assert!(!jobs.iter().any(|job| job.action == "update"));
        let payloads: Vec<JsonValue> = jobs[1..9]
            .iter()
            .map(|job| serde_json::from_str(job.payload.as_deref().unwrap()).unwrap())
            .collect();
        let combinations: Vec<_> = payloads
            .iter()
            .map(|p| {
                (
                    p["VERSION"].as_str().unwrap(),
                    p["playbook_name"].as_str().unwrap(),
                    p["ansible_limit"][0].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            combinations,
            vec![
                ("1", "service", "host-1"),
                ("1", "service", "host-2"),
                ("1", "exporter", "host-1"),
                ("1", "exporter", "host-2"),
                ("2", "service", "host-1"),
                ("2", "service", "host-2"),
                ("2", "exporter", "host-1"),
                ("2", "exporter", "host-2"),
            ]
        );
        // The user's existing three validation steps must never produce deploy or update jobs.
        let config = process_config(json!([
            {"name":"Jira", "driver":"jira", "action":"validate"},
            {"name":"CX", "driver":"cx", "action":"validate"},
            {"name":"OLAM", "driver":"olam", "action":"validate"}
        ]));
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[2].name, "OLAM");
        assert_eq!(jobs[2].target.as_deref(), Some("https://olam.test"));
        assert!(jobs[2].payload.is_none());
        assert!(jobs.iter().all(|job| job.action == "validate"));
        let config = process_config(
            json!([{"name":"Deploy only", "driver":"olam", "action":"deploy", "workflow":["approve","deploy"]}]),
        );
        assert!(
            generate_jobs(&input, &config)
                .unwrap()
                .iter()
                .all(|job| job.action == "deploy")
        );
    }

    #[test]
    fn optional_tickets_skip_only_their_integrations_and_keep_deployments() {
        let config = process_config(json!([
            {"name":"Jira", "driver":"jira", "action":"validate"},
            {"name":"CX", "driver":"cx", "action":"validate"},
            {"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["validate","approve","deploy"]},
            {"name":"Report Jira", "driver":"jira", "action":"update", "report":{"scope":"plan","comment_template":"{{plan_name}}"}},
            {"name":"Report CX", "driver":"cx", "action":"update", "report":{"scope":"plan","comment_template":"{{plan_name}}"}}
        ]));
        for (jira, cx) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut input = plan_input();
            if !jira { input.inputs.jira = None; }
            if !cx { input.inputs.cx = None; }
            let jobs = generate_jobs(&input, &config).unwrap();
            assert_eq!(jobs.iter().filter(|job| job.driver == "olam").count(), 8);
            assert_eq!(jobs.iter().filter(|job| job.driver == "jira").count(), if jira { 2 } else { 0 });
            assert_eq!(jobs.iter().filter(|job| job.driver == "cx").count(), if cx { 2 } else { 0 });
            let payload: JsonValue = serde_json::from_str(jobs.iter().find(|job| job.driver == "olam").unwrap().payload.as_ref().unwrap()).unwrap();
            assert_eq!(payload.get("ticket_system").and_then(JsonValue::as_str), if cx { Some("CX") } else if jira { Some("JIRA") } else { None });
        }
        let mut input = plan_input();
        input.inputs.jira.as_mut().unwrap().issue = " ".into();
        input.inputs.cx.as_mut().unwrap().request = " ".into();
        assert!(generate_jobs(&input, &config).unwrap().iter().all(|job| job.driver == "olam"));
    }

    #[test]
    fn inventory_groups_stay_in_one_payload_while_hosts_stay_separate() {
        let mut input = plan_input();
        let config = process_config(json!([
            {"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["validate","approve","deploy"]}
        ]));
        let host_jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(host_jobs.len(), 8);
        assert_eq!(host_jobs[0].target.as_deref(), Some("host-1"));
        assert_eq!(host_jobs[1].target.as_deref(), Some("host-2"));
        assert_eq!(
            serde_json::from_str::<JsonValue>(host_jobs[0].payload.as_deref().unwrap()).unwrap()["ansible_limit"],
            json!(["host-1"])
        );

        input.inputs.olam.targets.hosts.clear();
        input.inputs.olam.targets.inventory_groups = vec![" group-one ".into(), "group-two".into()];
        let grouped = generate_jobs(&input, &config).unwrap();
        assert_eq!(grouped.len(), 4); // Two steps x two roles, each receives both groups.
        assert!(grouped.iter().all(|job| job.action == "validate_deploy"
            && job.target.as_deref() == Some("group-one, group-two")));
        for (index, job) in grouped.iter().enumerate() {
            let payload: JsonValue = serde_json::from_str(job.payload.as_deref().unwrap()).unwrap();
            assert_eq!(payload["ansible_limit"], json!(["group-one", "group-two"]));
            assert_eq!(
                payload["playbook_name"],
                json!(if index % 2 == 0 {
                    "service"
                } else {
                    "exporter"
                })
            );
            assert_eq!(payload["VERSION"], json!(if index < 2 { "1" } else { "2" }));
            assert_eq!(payload["STRATEGY"], json!("linear"));
            assert_eq!(payload["ticket_system"], json!("CX"));
            assert_eq!(payload["ticket_id"], json!("REQ-1"));
            assert_eq!(payload["ansible_password"], json!("$encrypted$"));
        }
        input.inputs.olam.targets.roles.truncate(1);
        input.inputs.olam.steps.truncate(1);
        assert_eq!(generate_jobs(&input, &config).unwrap().len(), 1);
    }

    #[test]
    fn explicit_job_mode_splits_hosts_or_combines_them_per_step() {
        let config = process_config(json!([{"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["validate","approve","deploy"]}]));
        let mut input = plan_input();
        input.inputs.olam.targets.roles = vec!["service".into()];
        input.inputs.olam.targets.hosts = vec!["host-1".into(), "host-2".into(), "host-3".into()];
        input.inputs.olam.targets.job_mode = Some(OlamJobMode::PerHost);
        let separate = generate_jobs(&input, &config).unwrap();
        assert_eq!(separate.len(), 6);
        for (index, job) in separate.iter().enumerate() {
            assert_eq!(job.target.as_deref(), Some(["host-1", "host-2", "host-3"][index % 3]));
            let payload: JsonValue = serde_json::from_str(job.payload.as_ref().unwrap()).unwrap();
            assert_eq!(payload["ansible_limit"].as_array().unwrap().len(), 1);
            assert_eq!(payload["VERSION"], if index < 3 { "1" } else { "2" });
        }
        input.inputs.olam.targets.job_mode = Some(OlamJobMode::Together);
        let together = generate_jobs(&input, &config).unwrap();
        assert_eq!(together.len(), 2);
        for job in together {
            let payload: JsonValue = serde_json::from_str(job.payload.as_ref().unwrap()).unwrap();
            assert_eq!(payload["ansible_limit"], json!(["host-1","host-2","host-3"]));
        }
        input.inputs.olam.targets.hosts = vec!["host-1".into(), "host-3".into()];
        let partial = generate_jobs(&input, &config).unwrap();
        assert_eq!(partial.len(), 2);
        let payload: JsonValue = serde_json::from_str(partial[0].payload.as_ref().unwrap()).unwrap();
        assert_eq!(payload["ansible_limit"], json!(["host-1","host-3"]));
        input.inputs.olam.targets.hosts.clear();
        input.inputs.olam.targets.inventory_groups = vec!["site-group".into()];
        input.inputs.olam.targets.job_mode = Some(OlamJobMode::PerHost);
        assert!(validate_olam_input(&input).unwrap_err().to_string().contains("Per-host"));
        assert!(serde_yaml::from_str::<PlanInput>("plan_name: Test\nprocess: Deploy\ninputs:\n  olam:\n    targets:\n      job_mode: invalid").is_err());
    }

    #[test]
    fn grouped_target_input_requires_explicit_unambiguous_nonempty_groups() {
        let mut input = plan_input();
        let config = process_config(json!([
            {"name":"Deploy", "driver":"olam", "action":"deploy", "workflow":["validate","approve","deploy"]}
        ]));
        input.inputs.olam.targets.inventory_groups = vec!["group-one".into()];
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("not both")
        );
        input.inputs.olam.targets.hosts.clear();
        input.inputs.olam.targets.inventory_groups = vec![" ".into()];
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("inventory group must have a value")
        );
        input.inputs.olam.targets.inventory_groups.clear();
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("inventory_groups")
        );
    }

    #[test]
    fn explicit_workflow_creates_one_row_per_target_and_requires_configuration() {
        let input = plan_input();
        let mut config = process_config(json!([
            {"name":"Deploy", "driver":"olam", "action":"deploy"}
        ]));
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("workflow")
        );
        config["steps"][0]["workflow"] = json!(["validate", "approve", "deploy"]);
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 8); // Two plan steps x two roles x two hosts.
        assert!(
            jobs.iter()
                .all(|job| job.action == "validate_deploy" && job.details == "Validation pending")
        );
        assert_eq!(jobs[0].target.as_deref(), Some("host-1"));
        assert_eq!(jobs[1].target.as_deref(), Some("host-2"));
    }

    #[test]
    fn jira_report_uses_only_configured_step_and_freezes_its_templates_in_the_job() {
        let input = plan_input();
        let mut config =
            process_config(json!([{"name":"Update Jira","driver":"jira","action":"update"}]));
        assert!(generate_jobs(&input, &config).is_err());
        let report = json!({"scope":"deployment","comment_template":"Role {{role}} / {{target}}","attachments":[{"source":"deployment_log","filename_template":"{{job_id}}.log"}]});
        config["steps"][0]["report"] = report.clone();
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].target.as_deref(), Some("APP-1"));
        assert_eq!(
            serde_json::from_str::<JsonValue>(jobs[0].payload.as_deref().unwrap()).unwrap(),
            report
        );
    }

    #[test]
    fn cx_step_requesters_validate_and_freeze_per_step_with_plan_override() {
        let mut input = plan_input();
        let mut config = process_config(json!([
            {"name":"First CX update","driver":"cx","action":"update","requester":"first@example.test","report":{"scope":"plan","comment_template":"{{plan_name}}"}},
            {"name":"Second CX update","driver":"cx","action":"update","requester":"second@example.test","report":{"scope":"plan","comment_template":"{{plan_name}}"}}
        ]));
        config["details"]["cx"]
            .as_object_mut()
            .unwrap()
            .remove("requester");
        assert_eq!(generate_jobs(&input, &config).unwrap().len(), 2);
        let plan = "plan_name: Upgrade\nprocess: Deploy\ninputs:\n  cx: {request: REQ-1}\n";
        let frozen: JsonValue =
            serde_json::from_str(&process_with_plan_requester(&config.to_string(), plan).unwrap())
                .unwrap();
        assert_eq!(frozen["steps"][0]["requester"], "first@example.test");
        assert_eq!(frozen["steps"][1]["requester"], "second@example.test");
        assert!(frozen["details"]["cx"].get("requester").is_none());
        let override_plan = "plan_name: Upgrade\nprocess: Deploy\ninputs:\n  cx: {request: REQ-1, requester: plan@example.test}\n";
        let overridden: JsonValue = serde_json::from_str(
            &process_with_plan_requester(&config.to_string(), override_plan).unwrap(),
        )
        .unwrap();
        assert!(
            overridden["steps"]
                .as_array()
                .unwrap()
                .iter()
                .all(|step| step["requester"] == "plan@example.test")
        );
        assert_eq!(
            overridden["details"]["cx"]["requester"],
            "plan@example.test"
        );
        config["steps"].as_array_mut().unwrap().pop();
        let frozen: JsonValue =
            serde_json::from_str(&process_with_plan_requester(&config.to_string(), plan).unwrap())
                .unwrap();
        assert_eq!(frozen["details"]["cx"]["requester"], "first@example.test");
        input.inputs.cx.as_mut().unwrap().requester = Some("plan@example.test".into());
        assert_eq!(
            effective_cx_requester(&input, &config["steps"][0], &config["details"]["cx"]),
            Some("plan@example.test")
        );
        config["steps"][0]["requester"] = json!(123);
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("must be text")
        );
    }

    #[test]
    fn cx_report_is_explicit_frozen_and_requires_requester_and_request_input() {
        let mut input = plan_input();
        let mut config =
            process_config(json!([{"name":"Update CX","driver":"cx","action":"update"}]));
        assert!(generate_jobs(&input, &config).is_err());
        let report = json!({"scope":"plan","comment_template":"{{request}}\n{{deployments}}","deployment_template":"{{target}}: {{job_id}}","attachments":[{"source":"plan_log","compression":"zip","filename_template":"{{request}}.zip"}]});
        config["steps"][0]["report"] = report.clone();
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].target.as_deref(), Some("REQ-1"));
        assert_eq!(
            serde_json::from_str::<JsonValue>(jobs[0].payload.as_deref().unwrap()).unwrap(),
            report
        );
        config["details"]["cx"]["requester"] = json!("");
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("requester")
        );
        input.inputs.cx.as_mut().unwrap().requester = Some("plan-operator@example.test".into());
        assert!(generate_jobs(&input, &config).is_ok());
        let submitted = process_with_plan_requester(&config.to_string(), "plan_name: Upgrade\nprocess: Deploy\ninputs:\n  cx:\n    request: REQ-1\n    requester: plan-operator@example.test\n").unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&submitted).unwrap()["details"]["cx"]["requester"],
            "plan-operator@example.test"
        );
        config["details"]["cx"]["requester"] = json!("operator@example.test");
        input.inputs.cx = None;
        assert!(generate_jobs(&input, &config).unwrap().is_empty());
    }

    #[test]
    fn disabled_jobs_and_approve_all_survive_workspace_reopen_and_plan_regeneration() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-disabled-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        let config = process_config(json!([
            {"name":"Endpoint one", "driver":"olam", "action":"validate"},
            {"name":"Endpoint two", "driver":"olam", "action":"validate"}
        ]));
        state.with_connection(|c| { c.execute("INSERT INTO deployment_processes(id,name,definition_json) VALUES ('process','Deploy',?1)", [config.to_string()])?; Ok(()) }).unwrap();
        QueueService::create(
            &state,
            "plan_name: Check\nprocess: Deploy\ninputs: {}".into(),
        )
        .unwrap();
        let initial = QueueService::snapshot(&state).unwrap();
        let plan_id = initial.plans[0].id.clone();
        let job_id = initial.plans[0].jobs[0].id.clone();
        QueueService::set_approve_all(&state, &plan_id, true).unwrap();
        QueueService::control(&state, "job", &job_id, "disable").unwrap();
        state.lock().unwrap();
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        assert_eq!(
            QueueService::snapshot(&state).unwrap().plans[0].jobs[0].status,
            "disabled"
        );
        QueueService::prepare_plan_start(&state, &plan_id, true).unwrap();
        let submission = QueueService::agent_submission(&state, &plan_id, None).unwrap();
        assert!(submission.approve_all);
        assert!(QueueService::snapshot(&state).unwrap().plans[0].approve_all);
        QueueService::set_approve_all(&state, &plan_id, false).unwrap();
        assert!(!QueueService::agent_submission(&state, &plan_id, None).unwrap().approve_all);
        assert!(submission.jobs[0].disabled);
        assert!(!submission.jobs[1].disabled);
        assert_ne!(submission.jobs[0].id, job_id);
        QueueService::control(&state, "job", &submission.jobs[0].id, "enable").unwrap();
        assert!(
            !QueueService::agent_submission(&state, &plan_id, None)
                .unwrap()
                .jobs[0]
                .disabled
        );
        state.lock().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn compacts_only_never_started_per_host_endpoint_checks_from_the_frozen_process() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-endpoint-queue-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        let config =
            process_config(json!([{"name":"Validate OLAM", "driver":"olam", "action":"validate"}]));
        state.with_connection(|connection| {
            connection.execute("INSERT INTO deployment_processes(id,name,definition_json) VALUES ('process','Deploy',?1)", [config.to_string()])?;
            Ok(())
        }).unwrap();
        QueueService::create(
            &state,
            "plan_name: Check\nprocess: Deploy\ninputs: {}".into(),
        )
        .unwrap();
        let initial = QueueService::snapshot(&state).unwrap();
        let id = &initial.plans[0].id;
        assert_eq!(initial.plans[0].jobs.len(), 1);
        let first = &initial.plans[0].jobs[0];
        assert_eq!(first.target.as_deref(), Some("https://olam.test"));
        assert!(first.payload.is_none());
        state.with_connection(|connection| {
            connection.execute("UPDATE deployment_jobs SET payload_json='{}',target='host-1',status='completed',log_output='preserve history' WHERE plan_id=?1", [id])?;
            connection.execute("INSERT INTO deployment_jobs(id,plan_id,sequence,name,driver,action,target,payload_json) VALUES ('old-host-2',?1,2,'OLAM','olam','validate','host-2','{}')", [id])?;
            connection.execute("UPDATE deployment_processes SET definition_json=?1", [process_config(json!([{"name":"New config must wait", "driver":"olam", "action":"deploy", "workflow":["approve","deploy"]}])).to_string()])?;
            Ok(())
        }).unwrap();
        let history = QueueService::snapshot(&state).unwrap();
        assert_eq!(history.plans[0].jobs.len(), 2);
        assert_eq!(history.plans[0].jobs[0].log, "preserve history");
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE deployment_jobs SET status='queued',log_output='' WHERE plan_id=?1",
                    [id],
                )?;
                Ok(())
            })
            .unwrap();
        let compacted = QueueService::snapshot(&state).unwrap();
        assert_eq!(compacted.plans[0].jobs.len(), 1);
        assert_eq!(compacted.plans[0].jobs[0].name, "Validate OLAM");
        assert_eq!(
            compacted.plans[0].jobs[0].target.as_deref(),
            Some("https://olam.test")
        );
        assert!(compacted.plans[0].jobs[0].payload.is_none());
        assert_eq!(
            QueueService::snapshot(&state).unwrap().plans[0].jobs[0].id,
            compacted.plans[0].jobs[0].id
        );
        state.lock().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn validates_only_inputs_required_by_selected_steps_and_rejects_unknown_actions() {
        let input: PlanInput = serde_yaml::from_str(
            "plan_name: Check\nprocess: Deploy\ninputs:\n  jira: {issue: APP-1}",
        )
        .unwrap();
        validate_input(&input).unwrap();
        let mut config =
            process_config(json!([{"name":"Jira", "driver":"jira", "action":"validate"}]));
        let jobs = generate_jobs(&input, &config).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].target.as_deref(), Some("APP-1"));
        config["steps"][0]["driver"] = json!("cx");
        assert!(generate_jobs(&input, &config).unwrap().is_empty());
        config["steps"][0]["action"] = json!("unknown");
        assert!(
            generate_jobs(&input, &config)
                .unwrap_err()
                .to_string()
                .contains("Unsupported Process action")
        );
    }

    #[test]
    fn snapshots_freeze_process_until_restart_and_preserve_jobs_during_reads() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-process-snapshot-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        let old = process_config(json!([
            {"name":"First", "driver":"jira", "action":"validate"},
            {"name":"Second", "driver":"jira", "action":"validate"}
        ]));
        state.with_connection(|connection| {
            connection.execute("INSERT INTO deployment_processes(id,name,definition_json) VALUES ('process','Deploy',?1)", [old.to_string()])?;
            Ok(())
        }).unwrap();
        QueueService::create(
            &state,
            "plan_name: Check\nprocess: Deploy\ninputs:\n  jira: {issue: APP-1}".into(),
        )
        .unwrap();
        let before = QueueService::snapshot(&state).unwrap();
        let id = &before.plans[0].id;
        let first_id = &before.plans[0].jobs[0].id;
        let new =
            process_config(json!([{"name":"Only this", "driver":"jira", "action":"validate"}]));
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE deployment_processes SET definition_json = ?1",
                    [new.to_string()],
                )?;
                Ok(())
            })
            .unwrap();
        QueueService::prepare_plan_start(&state, id, false).unwrap();
        let submission = QueueService::agent_submission(&state, id, None).unwrap();
        assert_eq!(submission.jobs.len(), 2);
        assert_eq!(
            serde_json::from_str::<JsonValue>(&submission.process_definition).unwrap(),
            old
        );
        assert_eq!(
            &QueueService::snapshot(&state).unwrap().plans[0].jobs[0].id,
            first_id
        );
        for status in ["running", "awaiting_approval"] {
            state
                .with_connection(|connection| {
                    connection.execute(
                        "UPDATE deployment_plans SET status=?2 WHERE id=?1",
                        params![id, status],
                    )?;
                    Ok(())
                })
                .unwrap();
            assert!(QueueService::prepare_plan_start(&state, id, true).is_err());
            assert_eq!(
                QueueService::snapshot(&state).unwrap().plans[0].jobs.len(),
                2
            );
        }
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE deployment_plans SET status='stopped' WHERE id=?1",
                    [id],
                )?;
                Ok(())
            })
            .unwrap();
        QueueService::prepare_plan_start(&state, id, true).unwrap();
        let submission = QueueService::agent_submission(&state, id, None).unwrap();
        assert_eq!(submission.jobs.len(), 1);
        assert_eq!(submission.jobs[0].name, "Only this");
        assert_ne!(&submission.jobs[0].id, first_id);
        assert_eq!(
            serde_json::from_str::<JsonValue>(&submission.process_definition).unwrap(),
            new
        );
        // Legacy plans are left untouched by reads and rebuilt only when explicitly started.
        state.with_connection(|connection| {
            connection.execute("UPDATE deployment_plans SET process_definition_json=NULL WHERE id=?1", [id])?;
            connection.execute("UPDATE deployment_jobs SET driver='olam',action='validate',log_output='keep me' WHERE plan_id=?1", [id])?;
            Ok(())
        }).unwrap();
        assert_eq!(
            QueueService::snapshot(&state).unwrap().plans[0].jobs[0].log,
            "keep me"
        );
        assert_eq!(
            QueueService::snapshot(&state).unwrap().plans[0].jobs[0].action,
            "validate"
        );
        QueueService::prepare_plan_start(&state, id, false).unwrap();
        assert_eq!(
            QueueService::snapshot(&state).unwrap().plans[0].jobs[0].driver,
            "jira"
        );
        state.lock().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_details_follow_live_validation_approval_and_deployment_states() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-queue-state-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        state.with_connection(|connection| {
            connection.execute("INSERT INTO deployment_plans(id, name, process_name, definition_yaml, plan_number) VALUES ('plan', 'Upgrade', 'Deploy', '{}', 1)", [])?;
            connection.execute("INSERT INTO deployment_jobs(id, plan_id, sequence, name, driver, action, target, payload_json, details) VALUES ('job', 'plan', 1, 'Upgrade', 'olam', 'deploy', 'host-1', '{}', 'Validation pending')", [])?;
            Ok(())
        }).unwrap();
        let mut set = AgentJobSetStatus {
            approve_all: false,
            execution_revision: 2,
            credentials_required: false,
            id: "plan".into(),
            plan_number: 1,
            name: "Upgrade".into(),
            process_name: "Deploy".into(),
            plan_definition: "{}".into(),
            process_definition: "{}".into(),
            status: "queued".into(),
            jobs: vec![AgentJobStatus {
                id: "job".into(),
                sequence: 1,
                name: "Upgrade".into(),
                driver: "olam".into(),
                action: "deploy".into(),
                target: Some("host-1".into()),
                details: "Validation pending".into(),
                payload: Some("{}".into()),
                status: "queued".into(),
                log: String::new(),
                phase: Some("validation".into()),
                validation_job_id: Some(133),
                deployment_job_id: Some(137),
                finished_at: None,
                report_progress: Default::default(),
            }],
        };
        for (status, phase, expected) in [
            ("queued", "validation", "Validation pending"),
            ("running", "validation", "Validation running · Job #133"),
            (
                "awaiting_approval",
                "deployment",
                "Validation passed - awaiting approval · Job #133",
            ),
            ("running", "deployment", "Deployment running · Job #137"),
            ("completed", "deployment", "Deployment completed · Job #137"),
            ("failed", "deployment", "Deployment failed · Job #137"),
            ("failed", "validation", "Validation failed · Job #133"),
            ("warning", "validation", "Validation warning · Job #133"),
            ("stopped", "deployment", "Deployment stopped · Job #137"),
            ("denied", "deployment", "Deployment denied · Job #133"),
        ] {
            set.status = status.into();
            set.jobs[0].status = status.into();
            set.jobs[0].phase = Some(phase.into());
            set.jobs[0].log = format!("Current phase: {phase}; state: {status}");
            QueueService::synchronize_job_sets(&state, std::slice::from_ref(&set)).unwrap();
            let snapshot = QueueService::snapshot(&state).unwrap();
            assert_eq!(snapshot.plans[0].status, status);
            let job = &snapshot.plans[0].jobs[0];
            assert_eq!(job.details, expected);
            assert_eq!(job.status, status);
            assert_eq!(job.log, set.jobs[0].log);
            assert_eq!(job.payload.as_deref(), Some("{}"));
        }
        // Older Agents have no ID fields. Only use log IDs from the matching phase.
        set.jobs[0].validation_job_id = None;
        set.jobs[0].deployment_job_id = None;
        set.jobs[0].status = "running".into();
        set.jobs[0].phase = Some("deployment".into());
        set.jobs[0].log =
            "OLAM validation template 'Validate' (ID 1) completed successfully as Ansible job 133."
                .into();
        assert_eq!(agent_job_details(&set.jobs[0]), "Deployment running");
        set.jobs[0]
            .log
            .push_str("\nOLAM deployment started as Ansible job 137.");
        assert_eq!(
            agent_job_details(&set.jobs[0]),
            "Deployment running · Job #137"
        );
        set.jobs[0]
            .log
            .push_str("\nStarting OLAM deployment for host-1…");
        assert_eq!(agent_job_details(&set.jobs[0]), "Deployment running");
        set.jobs[0]
            .log
            .push_str("\nOLAM deployment started as Ansible job 138.");
        assert_eq!(
            agent_job_details(&set.jobs[0]),
            "Deployment running · Job #138"
        );
        set.jobs[0].status = "awaiting_approval".into();
        assert_eq!(
            agent_job_details(&set.jobs[0]),
            "Validation passed - awaiting approval · Job #133"
        );
        // Other drivers keep their own descriptions.
        set.jobs[0].driver = "jira".into();
        set.jobs[0].action = "validate".into();
        set.jobs[0].details = "Issue validation".into();
        assert_eq!(agent_job_details(&set.jobs[0]), "Issue validation");
        state.lock().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn creates_a_grouped_queue_from_optional_inputs_and_olam_steps() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-queue-test-{}-{unique}",
            std::process::id()
        ));
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
                [r#"{"steps":[{"name":"Validate Jira","driver":"jira","action":"validate"},{"name":"Validate CX","driver":"cx","action":"validate"},{"name":"Deploy OLAM","driver":"olam","action":"deploy","workflow":["approve","deploy"]},{"name":"Update Jira","driver":"jira","action":"update","report":{"scope":"deployment","comment_template":"{{target}}: {{status}}"}},{"name":"Update CX","driver":"cx","action":"update","report":{"scope":"plan","comment_template":"{{deployments}}","deployment_template":"{{target}}: {{status}}"}}],"details":{"jira":{},"cx":{"url":"https://cx.test/requests","account":"cx","requester":"operator@example.test"},"olam":{"account":"olam_api_account","payload_defaults":{"ENVIRONMENT":"test","escalation_contact":"team"}}}}"#],
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

        let created_id = QueueService::create(
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
        assert_eq!(snapshot.plans[0].id, created_id);
        assert_eq!(snapshot.plans[0].status, "queued");
        assert!(snapshot.plans[0].jobs.iter().all(|job| job.status == "queued"));
        assert_eq!(snapshot.plans[0].jobs.len(), 8);
        assert_eq!(snapshot.plans[0].jobs[0].name, "Validate Jira");
        assert_eq!(
            snapshot.plans[0].jobs[2].name,
            "Deploy OLAM · Upgrade phase one"
        );
        assert_eq!(snapshot.plans[0].jobs[2].target.as_deref(), Some("host-1"));
        assert_eq!(
            snapshot.plans[0].jobs[3].name,
            "Deploy OLAM · Upgrade phase one"
        );
        assert_eq!(snapshot.plans[0].jobs[3].target.as_deref(), Some("host-2"));
        assert_eq!(
            snapshot.plans[0].jobs[4].name,
            "Deploy OLAM · Upgrade phase two"
        );
        assert_eq!(snapshot.plans[0].jobs[4].target.as_deref(), Some("host-1"));
        assert_eq!(
            snapshot.plans[0].jobs[5].name,
            "Deploy OLAM · Upgrade phase two"
        );
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
        let before_restart =
            QueueService::snapshot(&state).expect("read queue without rewriting it");
        assert_eq!(before_restart.plans[0].jobs[2].target, None);
        QueueService::prepare_plan_start(&state, &plan_id, true).unwrap();
        let snapshot = QueueService::snapshot(&state).expect("regenerate only on explicit Restart");
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
