use std::{
    collections::{BTreeMap, BTreeSet},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};
use crate::storage::StorageError;

use super::model::{
    DeploymentConfigResource, DeploymentConfigSnapshot, DeploymentConfigValidation,
};

const DRIVER_CATALOG: &str = include_str!("drivers/catalog.json");

pub struct DeploymentsService;

impl DeploymentsService {
    pub fn snapshot(state: &WorkspaceState) -> Result<DeploymentConfigSnapshot, DeploymentError> {
        state
            .with_connection(|connection| {
                Ok(DeploymentConfigSnapshot {
                    drivers: load_resources(connection, ResourceType::Driver)?,
                    processes: load_resources(connection, ResourceType::Process)?,
                })
            })
            .map_err(DeploymentError::from)
    }

    pub fn validate(
        state: &WorkspaceState,
        resource_type: &str,
        definition: String,
        replacing_id: Option<&str>,
    ) -> Result<DeploymentConfigValidation, DeploymentError> {
        let kind = ResourceType::parse(resource_type)?;
        let parsed = state
            .with_connection(|connection| {
                let parsed = validate_definition(connection, kind, &definition, replacing_id)
                    .map_err(to_sql_error)?;
                if kind == ResourceType::Process {
                    validate_process_credentials(connection, &parsed.value)
                        .map_err(to_sql_error)?;
                }
                Ok(parsed)
            })
            .map_err(map_workspace_validation_error)?;
        for endpoint in &parsed.endpoints {
            validate_http_endpoint(endpoint)?;
        }
        Ok(DeploymentConfigValidation {
            formatted: parsed.formatted,
            id: parsed.id,
            name: parsed.name,
        })
    }

    pub fn save(
        state: &WorkspaceState,
        resource_type: &str,
        definition: String,
        replacing_id: Option<&str>,
    ) -> Result<DeploymentConfigResource, DeploymentError> {
        let kind = ResourceType::parse(resource_type)?;
        state
            .with_connection(|connection| {
                let parsed = validate_definition(connection, kind, &definition, replacing_id)
                    .map_err(to_sql_error)?;
                let existing = replacing_id
                    .map(|id| resource_by_id(connection, kind, id))
                    .transpose()?
                    .flatten();
                if replacing_id.is_some() && existing.is_none() {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }

                let transaction = connection.unchecked_transaction()?;
                if let Some(current) = &existing {
                    archive_resource(&transaction, kind, current)?;
                    if kind == ResourceType::Driver {
                        transaction.execute(
                            "UPDATE deployment_tools SET name = ?2, definition_json = ?3,
                             transport = ?4, revision = revision + 1,
                             updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                            params![parsed.id, parsed.name, parsed.formatted, parsed.transport],
                        )?;
                    } else {
                        transaction.execute(
                            "UPDATE deployment_processes SET name = ?2, definition_json = ?3,
                             revision = revision + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                            params![parsed.id, parsed.name, parsed.formatted],
                        )?;
                    }
                } else if kind == ResourceType::Driver {
                    transaction.execute(
                        "INSERT INTO deployment_tools(id, name, transport, definition_json) VALUES (?1, ?2, ?3, ?4)",
                        params![parsed.id, parsed.name, parsed.transport, parsed.formatted],
                    )?;
                } else {
                    transaction.execute(
                        "INSERT INTO deployment_processes(id, name, definition_json) VALUES (?1, ?2, ?3)",
                        params![parsed.id, parsed.name, parsed.formatted],
                    )?;
                }
                transaction.commit()?;
                resource_by_id(connection, kind, &parsed.id)?
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(map_workspace_validation_error)
    }

    pub fn delete(
        state: &WorkspaceState,
        resource_type: &str,
        id: &str,
    ) -> Result<(), DeploymentError> {
        let kind = ResourceType::parse(resource_type)?;
        state
            .with_connection(|connection| {
                let resource = resource_by_id(connection, kind, id)?
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                if kind == ResourceType::Driver {
                    let users = processes_using_tool(connection, id).map_err(to_sql_error)?;
                    if !users.is_empty() {
                        return Err(to_sql_error(DeploymentError::DriverInUse(
                            id.to_owned(),
                            users.join(", "),
                        )));
                    }
                }
                let transaction = connection.unchecked_transaction()?;
                archive_resource(&transaction, kind, &resource)?;
                transaction
                    .execute(&format!("DELETE FROM {} WHERE id = ?1", kind.table()), [id])?;
                transaction.commit()
            })
            .map_err(map_workspace_validation_error)
    }

    pub fn duplicate(
        state: &WorkspaceState,
        resource_type: &str,
        id: &str,
    ) -> Result<DeploymentConfigResource, DeploymentError> {
        let kind = ResourceType::parse(resource_type)?;
        state
            .with_connection(|connection| {
                let resource = resource_by_id(connection, kind, id)?
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                let mut value: Value = serde_json::from_str(&resource.definition)
                    .map_err(|error| to_sql_error(DeploymentError::Json(error)))?;
                let object = value.as_object_mut().ok_or_else(|| {
                    to_sql_error(DeploymentError::Invalid("Definition must be a JSON object.".into()))
                })?;
                let copy_id = unique_copy_id(connection, kind, id)?;
                let copy_name = unique_copy_name(connection, kind, &resource.name)?;
                object.insert("id".into(), Value::String(copy_id));
                object.insert("name".into(), Value::String(copy_name));
                let definition = serde_json::to_string_pretty(&value)
                    .map_err(|error| to_sql_error(DeploymentError::Json(error)))?;
                let parsed = validate_definition(connection, kind, &definition, None)
                    .map_err(to_sql_error)?;
                if kind == ResourceType::Driver {
                    connection.execute(
                        "INSERT INTO deployment_tools(id, name, transport, definition_json) VALUES (?1, ?2, ?3, ?4)",
                        params![parsed.id, parsed.name, parsed.transport, parsed.formatted],
                    )?;
                } else {
                    connection.execute(
                        "INSERT INTO deployment_processes(id, name, definition_json) VALUES (?1, ?2, ?3)",
                        params![parsed.id, parsed.name, parsed.formatted],
                    )?;
                }
                resource_by_id(connection, kind, &parsed.id)?
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(map_workspace_validation_error)
    }

    pub fn restore_previous(
        state: &WorkspaceState,
        resource_type: &str,
        id: &str,
    ) -> Result<DeploymentConfigResource, DeploymentError> {
        let kind = ResourceType::parse(resource_type)?;
        let previous = state
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT definition_json FROM deployment_config_history
                         WHERE resource_type = ?1 AND resource_id = ?2
                         ORDER BY created_at DESC, rowid DESC LIMIT 1",
                        params![kind.as_str(), id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .map_err(DeploymentError::from)?
            .ok_or(DeploymentError::NoPreviousVersion)?;
        Self::save(state, resource_type, previous, Some(id))
    }

    pub fn bundle(state: &WorkspaceState) -> Result<String, DeploymentError> {
        let snapshot = Self::snapshot(state)?;
        let processes = definitions_as_values(&snapshot.processes)?;
        serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "processes": processes
        }))
        .map_err(DeploymentError::Json)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceType {
    Driver,
    Process,
}

impl ResourceType {
    fn parse(value: &str) -> Result<Self, DeploymentError> {
        match value {
            "driver" | "tool" => Ok(Self::Driver),
            "process" => Ok(Self::Process),
            _ => Err(DeploymentError::InvalidResourceType),
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::Driver => "deployment_tools",
            Self::Process => "deployment_processes",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Driver => "tool",
            Self::Process => "process",
        }
    }
}

struct ParsedResource {
    id: String,
    name: String,
    transport: String,
    formatted: String,
    value: Value,
    endpoints: Vec<String>,
}

fn validate_definition(
    connection: &Connection,
    kind: ResourceType,
    definition: &str,
    replacing_id: Option<&str>,
) -> Result<ParsedResource, DeploymentError> {
    let value: Value = serde_json::from_str(definition)?;
    let object = value
        .as_object()
        .ok_or_else(|| DeploymentError::Invalid("Definition must be a JSON object.".into()))?;
    let name = required_string(object, "name")?;
    let id = if let Some(id) = object.get("id").and_then(Value::as_str) {
        let id = id.trim();
        validate_identifier(id, "id")?;
        id.to_owned()
    } else if kind == ResourceType::Process {
        replacing_id
            .map(str::to_owned)
            .unwrap_or_else(|| identifier_from_name(&name))
    } else {
        return Err(DeploymentError::Invalid(
            "'id' must be a non-empty string.".into(),
        ));
    };
    if let Some(replacing_id) = replacing_id
        && replacing_id != id
    {
        return Err(DeploymentError::Invalid(
            "The id of an existing resource cannot be changed. Duplicate it to create a new id."
                .into(),
        ));
    }

    let (transport, endpoints) = if kind == ResourceType::Driver {
        let transport = effective_tool_transport(object)?;
        let actions = effective_tool_actions(object)?;
        if actions.is_empty() {
            return Err(DeploymentError::Invalid(
                "Driver actions must contain at least one action.".into(),
            ));
        }
        for (action, definition) in &actions {
            validate_identifier(action, "action")?;
            if !definition.is_object() {
                return Err(DeploymentError::Invalid(format!(
                    "Driver action '{action}' must be a JSON object."
                )));
            }
        }
        (transport, Vec::new())
    } else {
        let endpoints = validate_process(connection, object)?;
        (String::new(), endpoints)
    };

    if kind == ResourceType::Driver && replacing_id.is_some() {
        validate_tool_update_against_processes(connection, &id, object)?;
    }
    let formatted = serde_json::to_string_pretty(&value)?;
    Ok(ParsedResource {
        id,
        name,
        transport,
        formatted,
        value,
        endpoints,
    })
}

fn validate_tool_credential(connection: &Connection, tool: &Value) -> Result<(), DeploymentError> {
    let credential = tool
        .get("authentication")
        .and_then(Value::as_object)
        .and_then(|authentication| authentication.get("credential"))
        .and_then(Value::as_str)
        .or_else(|| tool.get("credential").and_then(Value::as_str))
        .or_else(|| tool.get("account").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(credential) = credential else {
        return Ok(());
    };
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM vault_entries WHERE name = ?1 COLLATE NOCASE)",
        [credential],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(DeploymentError::MissingCredential(credential.to_owned()))
    }
}

fn validate_http_endpoint(base_url: &str) -> Result<(), DeploymentError> {
    let url =
        Url::parse(base_url).map_err(|error| DeploymentError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DeploymentError::Invalid(
            "HTTP Driver base_url must use http or https.".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| DeploymentError::InvalidUrl("host is missing".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| DeploymentError::InvalidUrl("port is missing".into()))?;
    let addresses = (host, port).to_socket_addrs().map_err(|error| {
        DeploymentError::EndpointUnavailable(base_url.into(), error.to_string())
    })?;
    let mut failure = "connection timed out".to_owned();
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(4)) {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(error) => failure = error.to_string(),
        }
    }
    Err(DeploymentError::EndpointUnavailable(
        base_url.into(),
        failure,
    ))
}

pub(super) fn olam_deploy_action(step: &Value) -> Result<&'static str, DeploymentError> {
    let phases = step.get("workflow").and_then(Value::as_array)
        .ok_or_else(|| DeploymentError::Invalid("Ansible deploy requires an explicit workflow, for example [\"validate\", \"approve\", \"deploy\"].".into()))?;
    let phases: Option<Vec<&str>> = phases.iter().map(Value::as_str).collect();
    match phases.as_deref() {
        Some(["validate", "approve", "deploy"]) => Ok("validate_deploy"),
        Some(["approve", "deploy"]) => Ok("deploy"),
        _ => Err(DeploymentError::Invalid("Supported Ansible workflows: [\"validate\", \"approve\", \"deploy\"] or [\"approve\", \"deploy\"].".into())),
    }
}

fn validate_process(
    _connection: &Connection,
    object: &Map<String, Value>,
) -> Result<Vec<String>, DeploymentError> {
    let details = object
        .get("details")
        .and_then(Value::as_object)
        .ok_or_else(|| DeploymentError::Invalid("Process details must be a JSON object.".into()))?;
    if details.is_empty() {
        return Err(DeploymentError::Invalid(
            "Process details must contain at least one configured driver.".into(),
        ));
    }
    for step in object
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if step["driver"] == "cx"
            && step["action"] == "update"
            && step
                .get("requester")
                .is_some_and(|value| !value.is_string())
        {
            return Err(DeploymentError::Invalid(
                "CX update step requester must be text.".into(),
            ));
        }
    }
    let catalog: Value = serde_json::from_str(DRIVER_CATALOG)
        .map_err(|error| DeploymentError::Invalid(format!("Driver catalog is invalid: {error}")))?;
    let mut endpoints = Vec::new();
    for (alias, driver_details) in details {
        validate_identifier(alias, "driver alias")?;
        if catalog.get(alias).and_then(Value::as_object).is_none() {
            return Err(DeploymentError::UnknownDriver(alias.to_owned()));
        }
        let driver_details = driver_details.as_object().ok_or_else(|| {
            DeploymentError::Invalid(format!("Process details '{alias}' must be a JSON object."))
        })?;
        let used_actions: Vec<&str> = object
            .get("steps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|step| step.get("driver").and_then(Value::as_str) == Some(alias))
            .filter_map(|step| step.get("action").and_then(Value::as_str))
            .collect();
        if alias == "container" {
            crate::agent_ipc::validate_container_config(&Value::Object(driver_details.clone()))
                .map_err(DeploymentError::Invalid)?;
            continue;
        }
        let url = required_string(driver_details, "url")?;
        if !used_actions.is_empty() {
            endpoints.push(url);
        }
        if alias == "cx" && used_actions.contains(&"update") {
            required_string(driver_details, "account")?;
            if driver_details
                .get("requester")
                .is_some_and(|value| !value.is_string())
            {
                return Err(DeploymentError::Invalid("CX requester must be text; configure it in inputs.cx.requester, the CX update step, or details.cx.requester.".into()));
            }
        }
        if alias == "olam" && used_actions.iter().any(|action| *action != "validate") {
            let combined = object["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|step| step["driver"] == "olam" && step["action"] == "deploy")
                .map(olam_deploy_action)
                .collect::<Result<Vec<_>, _>>()?
                .contains(&"validate_deploy");
            let payload_defaults = driver_details
                .get("payload_defaults")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    DeploymentError::Invalid(
                        "Process details 'olam.payload_defaults' must be a JSON object.".into(),
                    )
                })?;
            required_string(payload_defaults, "ENVIRONMENT")?;
            for (action, phase) in [
                ("validate_deployment", "validation"),
                ("deploy", "deployment"),
            ] {
                if used_actions.contains(&action) || (phase == "validation" && combined) {
                    required_string(driver_details, &format!("{phase}_template_job"))?;
                    required_positive_id(driver_details, &format!("{phase}_template_id"))?;
                }
            }
        }
    }

    let steps = object
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| DeploymentError::Invalid("Process steps must be a JSON array.".into()))?;
    if steps.is_empty() {
        return Err(DeploymentError::Invalid(
            "Process steps must contain at least one step.".into(),
        ));
    }
    for (index, step) in steps.iter().enumerate() {
        let step = step.as_object().ok_or_else(|| {
            DeploymentError::Invalid(format!("Process step {} must be a JSON object.", index + 1))
        })?;
        required_string(step, "name")?;
        let driver_name = required_identifier(step, "driver")?;
        if !details.contains_key(&driver_name) {
            return Err(DeploymentError::Invalid(format!(
                "Process step {} references missing details '{driver_name}'.",
                index + 1
            )));
        }
        let action_name = required_identifier(step, "action")?;
        if matches!(driver_name.as_str(), "olam" | "container") && action_name == "deploy" {
            olam_deploy_action(&Value::Object(step.clone()))?;
        }
        if matches!(driver_name.as_str(), "jira" | "cx") && action_name == "update" {
            let report = step.get("report").ok_or_else(|| {
                DeploymentError::Invalid(
                    "Update requires an explicit report in the Process step.".into(),
                )
            })?;
            (if driver_name == "cx" {
                crate::agent_ipc::validate_cx_update_report
            } else {
                crate::agent_ipc::validate_jira_update_report
            })(report)
            .map_err(DeploymentError::Invalid)?;
        }
        let available_actions = catalog
            .get(&driver_name)
            .and_then(|driver| driver.get("actions"))
            .and_then(Value::as_object)
            .ok_or_else(|| DeploymentError::UnknownDriver(driver_name.clone()))?;
        if !available_actions.contains_key(&action_name) {
            return Err(DeploymentError::Invalid(format!(
                "Process step {} references unsupported action '{action_name}' on Driver '{driver_name}'.",
                index + 1
            )));
        }
    }
    Ok(endpoints)
}

fn validate_process_credentials(
    connection: &Connection,
    process: &Value,
) -> Result<(), DeploymentError> {
    let Some(details) = process.get("details").and_then(Value::as_object) else {
        return Ok(());
    };
    for driver in process
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|step| step.get("driver").and_then(Value::as_str))
    {
        if let Some(driver_details) = details.get(driver) {
            validate_tool_credential(connection, driver_details)?;
        }
    }
    Ok(())
}

fn validate_tool_update_against_processes(
    connection: &Connection,
    tool_id: &str,
    tool: &Map<String, Value>,
) -> Result<(), DeploymentError> {
    let catalog = tool_action_catalog(connection, Some((tool_id, tool)))?;
    for process in load_resources(connection, ResourceType::Process)? {
        let value: Value = serde_json::from_str(&process.definition)?;
        let steps = value
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for step in steps {
            let Some(step) = step.as_object() else {
                continue;
            };
            let Some(referenced_driver) = step
                .get("driver")
                .or_else(|| step.get("tool"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Some(action) = step.get("action").and_then(Value::as_str) else {
                continue;
            };
            if !catalog
                .get(referenced_driver)
                .is_some_and(|actions| actions.contains(action))
            {
                return Err(DeploymentError::Invalid(format!(
                    "Driver change would break Process '{}': action '{}.{}' would no longer exist.",
                    process.name, referenced_driver, action
                )));
            }
        }
    }
    Ok(())
}

fn tool_action_catalog(
    connection: &Connection,
    replacement: Option<(&str, &Map<String, Value>)>,
) -> Result<BTreeMap<String, BTreeSet<String>>, DeploymentError> {
    let mut catalog = BTreeMap::new();
    for tool in load_resources(connection, ResourceType::Driver)? {
        let value: Value = serde_json::from_str(&tool.definition)?;
        let actions = effective_tool_actions(value.as_object().ok_or_else(|| {
            DeploymentError::Invalid(format!("Driver '{}' has an invalid definition.", tool.name))
        })?)?
        .keys()
        .cloned()
        .collect();
        catalog.insert(tool.id, actions);
    }
    if let Some((id, definition)) = replacement {
        let actions = effective_tool_actions(definition)?
            .keys()
            .cloned()
            .collect();
        catalog.insert(id.to_owned(), actions);
    }
    Ok(catalog)
}

fn effective_tool_transport(tool: &Map<String, Value>) -> Result<String, DeploymentError> {
    if let Some(transport) = tool.get("transport").and_then(Value::as_str) {
        let transport = transport.trim();
        validate_identifier(transport, "transport")?;
        return Ok(transport.to_owned());
    }
    let driver = tool_driver(tool)?;
    required_identifier(&driver, "transport")
}

fn effective_tool_actions(
    tool: &Map<String, Value>,
) -> Result<Map<String, Value>, DeploymentError> {
    let mut actions = if tool.get("type").is_some() || tool.get("driver").is_some() {
        tool_driver(tool)?
            .get("actions")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| DeploymentError::Invalid("Driver actions are invalid.".into()))?
    } else {
        Map::new()
    };
    if let Some(overrides) = tool.get("actions") {
        let overrides = overrides.as_object().ok_or_else(|| {
            DeploymentError::Invalid("Driver actions must be a JSON object.".into())
        })?;
        actions.extend(overrides.clone());
    }
    if actions.is_empty() {
        return Err(DeploymentError::Invalid(
            "Driver needs a type or at least one action.".into(),
        ));
    }
    Ok(actions)
}

fn tool_driver(tool: &Map<String, Value>) -> Result<Map<String, Value>, DeploymentError> {
    let driver_name = tool
        .get("type")
        .or_else(|| tool.get("driver"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DeploymentError::Invalid("'type' must be a non-empty string.".into()))?
        .to_owned();
    validate_identifier(&driver_name, "type")?;
    let catalog: Value = serde_json::from_str(DRIVER_CATALOG)
        .map_err(|error| DeploymentError::Invalid(format!("Driver catalog is invalid: {error}")))?;
    catalog
        .get(&driver_name)
        .and_then(Value::as_object)
        .cloned()
        .ok_or(DeploymentError::UnknownDriver(driver_name))
}

fn processes_using_tool(
    connection: &Connection,
    tool_id: &str,
) -> Result<Vec<String>, DeploymentError> {
    let mut users = Vec::new();
    for process in load_resources(connection, ResourceType::Process)? {
        let value: Value = serde_json::from_str(&process.definition)?;
        let used = value
            .get("steps")
            .and_then(Value::as_array)
            .is_some_and(|steps| {
                steps.iter().any(|step| {
                    step.get("driver")
                        .or_else(|| step.get("tool"))
                        .and_then(Value::as_str)
                        == Some(tool_id)
                })
            });
        if used {
            users.push(process.name);
        }
    }
    Ok(users)
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, DeploymentError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DeploymentError::Invalid(format!("'{key}' must be a non-empty string.")))?;
    Ok(value.to_owned())
}

fn required_positive_id(object: &Map<String, Value>, key: &str) -> Result<i64, DeploymentError> {
    let value = object.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    });
    value
        .filter(|id| *id > 0)
        .ok_or_else(|| DeploymentError::Invalid(format!("'{key}' must be a positive template ID.")))
}

fn required_identifier(object: &Map<String, Value>, key: &str) -> Result<String, DeploymentError> {
    let value = required_string(object, key)?;
    validate_identifier(&value, key)?;
    Ok(value)
}

fn identifier_from_name(name: &str) -> String {
    let mut identifier = String::new();
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !identifier.is_empty() {
                identifier.push('-');
            }
            identifier.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if identifier.is_empty() {
        "process".into()
    } else {
        identifier
    }
}

fn validate_identifier(value: &str, field: &str) -> Result<(), DeploymentError> {
    let valid = value.chars().enumerate().all(|(index, character)| {
        character.is_ascii_alphanumeric() || (index > 0 && matches!(character, '-' | '_' | '.'))
    });
    if valid {
        Ok(())
    } else {
        Err(DeploymentError::Invalid(format!(
            "'{field}' must use letters, numbers, dots, underscores or hyphens and start with a letter or number."
        )))
    }
}

fn load_resources(
    connection: &Connection,
    kind: ResourceType,
) -> rusqlite::Result<Vec<DeploymentConfigResource>> {
    let mut statement = connection.prepare(&format!(
        "SELECT id, name, definition_json, revision, created_at, updated_at FROM {} ORDER BY name COLLATE NOCASE",
        kind.table()
    ))?;
    statement
        .query_map([], resource_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn resource_by_id(
    connection: &Connection,
    kind: ResourceType,
    id: &str,
) -> rusqlite::Result<Option<DeploymentConfigResource>> {
    connection
        .query_row(
            &format!(
                "SELECT id, name, definition_json, revision, created_at, updated_at FROM {} WHERE id = ?1",
                kind.table()
            ),
            [id],
            resource_from_row,
        )
        .optional()
}

fn resource_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeploymentConfigResource> {
    Ok(DeploymentConfigResource {
        id: row.get(0)?,
        name: row.get(1)?,
        definition: row.get(2)?,
        revision: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn archive_resource(
    connection: &Connection,
    kind: ResourceType,
    resource: &DeploymentConfigResource,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO deployment_config_history(id, resource_type, resource_id, definition_json, revision)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            Uuid::new_v4().to_string(),
            kind.as_str(),
            resource.id,
            resource.definition,
            resource.revision
        ],
    )?;
    Ok(())
}

fn unique_copy_id(
    connection: &Connection,
    kind: ResourceType,
    source: &str,
) -> rusqlite::Result<String> {
    for number in 1..10_000 {
        let candidate = if number == 1 {
            format!("{source}-copy")
        } else {
            format!("{source}-copy-{number}")
        };
        if resource_by_id(connection, kind, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn unique_copy_name(
    connection: &Connection,
    kind: ResourceType,
    source: &str,
) -> rusqlite::Result<String> {
    for number in 1..10_000 {
        let candidate = if number == 1 {
            format!("{source} Copy")
        } else {
            format!("{source} Copy {number}")
        };
        let exists: bool = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE name = ?1 COLLATE NOCASE)",
                kind.table()
            ),
            [&candidate],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(candidate);
        }
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn definitions_as_values(
    resources: &[DeploymentConfigResource],
) -> Result<Vec<Value>, DeploymentError> {
    resources
        .iter()
        .map(|resource| serde_json::from_str(&resource.definition).map_err(DeploymentError::Json))
        .collect()
}

fn to_sql_error(error: DeploymentError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn map_workspace_validation_error(error: WorkspaceError) -> DeploymentError {
    if let WorkspaceError::Storage(StorageError::Database(
        rusqlite::Error::ToSqlConversionFailure(source),
    )) = error
    {
        return match source.downcast::<DeploymentError>() {
            Ok(error) => *error,
            Err(source) => DeploymentError::Invalid(source.to_string()),
        };
    }
    DeploymentError::Workspace(error)
}

#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("Unknown deployment configuration type.")]
    InvalidResourceType,
    #[error("{0}")]
    Invalid(String),
    #[error("Driver '{0}' is used by: {1}.")]
    DriverInUse(String, String),
    #[error("No previous version is available.")]
    NoPreviousVersion,
    #[error("Unknown Driver type '{0}'.")]
    UnknownDriver(String),
    #[error("Vault credential '{0}' does not exist.")]
    MissingCredential(String),
    #[error("Invalid Driver URL: {0}")]
    InvalidUrl(String),
    #[error("Driver endpoint '{0}' is unavailable: {1}")]
    EndpointUnavailable(String, String),
    #[error("JSON error at line {line}, column {column}: {message}", line = .0.line(), column = .0.column(), message = .0)]
    Json(#[from] serde_json::Error),
    #[error("Encrypted deployment storage error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::modules::workspace::WorkspaceService;

    use super::*;

    fn workspace() -> (WorkspaceState, std::path::PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-deployments-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .expect("create encrypted workspace");
        (state, directory)
    }

    #[test]
    fn container_process_requires_no_olam_url_or_vault_account() {
        let connection=Connection::open_in_memory().unwrap();
        let mut config:Value=serde_json::from_str(include_str!("../../../../docs/examples/devcorp-test-container.json")).unwrap();
        assert!(validate_process(&connection,config.as_object().unwrap()).unwrap().is_empty());
        config["details"]["container"]["image"]=json!("registry.test/image:next-version");
        validate_process(&connection,config.as_object().unwrap()).unwrap();
        config["steps"][0]["workflow"]=json!(["deploy"]);
        assert!(validate_process(&connection,config.as_object().unwrap()).is_err());
    }

    #[test]
    fn requires_only_the_olam_template_selected_by_process_steps() {
        let connection = Connection::open_in_memory().unwrap();
        for (action, phase) in [
            ("validate_deployment", "validation"),
            ("deploy", "deployment"),
        ] {
            let mut value = json!({"name":"Process", "steps":[{"name":"Step", "driver":"olam", "action":action}],
                "details":{"olam":{"url":"https://olam.test", "payload_defaults":{"ENVIRONMENT":"test"}}}});
            if action == "deploy" {
                value["steps"][0]["workflow"] = json!(["approve", "deploy"]);
            }
            value["details"]["olam"][format!("{phase}_template_job")] = json!("Template");
            value["details"]["olam"][format!("{phase}_template_id")] = json!(12);
            assert!(validate_process(&connection, value.as_object().unwrap()).is_ok());
        }
        let value = json!({"name":"Process", "steps":[{"name":"Check", "driver":"jira", "action":"validate"}],
            "details":{"jira":{"url":"https://jira.test"}, "olam":{"url":"https://unused.test"}}});
        let endpoints = validate_process(&connection, value.as_object().unwrap()).unwrap();
        assert_eq!(endpoints, vec!["https://jira.test"]);
    }

    #[test]
    fn combined_workflow_requires_both_templates_and_rejects_implicit_or_invalid_order() {
        let connection = Connection::open_in_memory().unwrap();
        let mut value = json!({"steps":[{"name":"Deploy", "driver":"olam", "action":"deploy"}],
            "details":{"olam":{"url":"https://olam.test", "payload_defaults":{"ENVIRONMENT":"test"},
                "deployment_template_id":12, "deployment_template_job":"Deploy"}}});
        assert!(
            validate_process(&connection, value.as_object().unwrap())
                .unwrap_err()
                .to_string()
                .contains("workflow")
        );
        value["steps"][0]["workflow"] = json!(["validate", "approve", "deploy"]);
        assert!(validate_process(&connection, value.as_object().unwrap()).is_err());
        value["details"]["olam"]["validation_template_id"] = json!(14);
        value["details"]["olam"]["validation_template_job"] = json!("Validate");
        assert!(validate_process(&connection, value.as_object().unwrap()).is_ok());
        for workflow in [
            json!([]),
            json!(["deploy", "validate"]),
            json!(["validate", "deploy"]),
            json!(["validate", 1, "deploy"]),
        ] {
            value["steps"][0]["workflow"] = workflow;
            assert!(validate_process(&connection, value.as_object().unwrap()).is_err());
        }
    }

    #[test]
    fn jira_update_requires_explicit_valid_report_configuration() {
        let connection = Connection::open_in_memory().unwrap();
        let mut config = json!({"steps":[{"name":"Report","driver":"jira","action":"update"}],"details":{"jira":{"url":"https://jira.test"}}});
        assert!(
            validate_process(&connection, config.as_object().unwrap())
                .unwrap_err()
                .to_string()
                .contains("report")
        );
        config["steps"][0]["report"] =
            json!({"scope":"deployment","comment_template":"Role {{role}}, job {{job_id}}"});
        assert!(validate_process(&connection, config.as_object().unwrap()).is_ok());
        config["steps"][0]["report"]["comment_template"] = json!("{{unknown}}");
        assert!(validate_process(&connection, config.as_object().unwrap()).is_err());
    }

    #[test]
    fn cx_update_requires_requester_and_valid_explicit_report() {
        let connection = Connection::open_in_memory().unwrap();
        let mut config = json!({"steps":[{"name":"Update CX","driver":"cx","action":"update"}],"details":{"cx":{"url":"https://cx.test/requests","account":"cx"}}});
        assert!(
            validate_process(&connection, config.as_object().unwrap())
                .unwrap_err()
                .to_string()
                .contains("report")
        );
        config["details"]["cx"]["requester"] = json!("operator@example.test");
        assert!(validate_process(&connection, config.as_object().unwrap()).is_err());
        config["steps"][0]["report"] =
            json!({"scope":"plan","comment_template":"Request {{request}}"});
        assert!(validate_process(&connection, config.as_object().unwrap()).is_ok());
        config["details"]["cx"]
            .as_object_mut()
            .unwrap()
            .remove("requester");
        assert!(validate_process(&connection, config.as_object().unwrap()).is_ok());
        config["steps"][0]["requester"] = json!("step@example.test");
        assert!(validate_process(&connection, config.as_object().unwrap()).is_ok());
        config["steps"][0]["requester"] = json!(42);
        assert!(
            validate_process(&connection, config.as_object().unwrap())
                .unwrap_err()
                .to_string()
                .contains("must be text")
        );
        config["steps"][0]["requester"] = json!("step@example.test");
        config["steps"][0]["report"]["comment_template"] = json!("{{typo}}");
        assert!(validate_process(&connection, config.as_object().unwrap()).is_err());
    }

    #[test]
    fn stores_versioned_processes_and_validates_bundled_driver_actions() {
        let (state, directory) = workspace();
        let missing_credential = DeploymentsService::validate(
            &state,
            "process",
            r#"{
              "name": "Production Deployment",
              "steps": [
                { "name": "Validate Jira", "driver": "jira", "action": "validate" }
              ],
              "details": {
                "jira": { "url": "https://jira.test", "account": "jira_account" }
              }
            }"#
            .to_owned(),
            None,
        )
        .expect_err("missing Vault reference must fail live validation");
        assert!(missing_credential.to_string().contains("jira_account"));

        let process = DeploymentsService::save(
            &state,
            "process",
            r#"{
              "name": "Production Deployment",
              "steps": [
                { "name": "Validate Jira", "driver": "jira", "action": "validate" }
              ],
              "details": {
                "jira": { "url": "https://jira.test", "account": "jira_account" }
              }
            }"#
            .to_owned(),
            None,
        )
        .expect("save process");
        assert_eq!(process.revision, 1);

        let broken = DeploymentsService::save(
            &state,
            "process",
            r#"{
              "name": "Broken Process",
              "steps": [
                { "name": "Bad", "driver": "jira", "action": "missing_action" }
              ],
              "details": {
                "jira": { "url": "https://jira.test", "account": "jira_account" }
              }
            }"#
            .to_owned(),
            None,
        )
        .expect_err("unknown bundled action must fail");
        assert!(broken.to_string().contains("missing_action"));

        let updated = DeploymentsService::save(
            &state,
            "process",
            r#"{
              "name": "Production Deployment Updated",
              "steps": [
                { "name": "Update Jira", "driver": "jira", "action": "update", "report": { "scope": "deployment", "comment_template": "{{target}}: {{status}}" } }
              ],
              "details": {
                "jira": { "url": "https://jira.test", "account": "jira_account" }
              }
            }"#
            .to_owned(),
            Some("production-deployment"),
        )
        .expect("update process");
        assert_eq!(updated.revision, 2);

        let restored =
            DeploymentsService::restore_previous(&state, "process", "production-deployment")
                .expect("restore process");
        assert_eq!(restored.revision, 3);
        assert_eq!(restored.name, "Production Deployment");

        let bundle = DeploymentsService::bundle(&state).expect("export bundle");
        assert!(bundle.contains("Validate Jira"));
        assert!(bundle.contains("Production Deployment"));
        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
