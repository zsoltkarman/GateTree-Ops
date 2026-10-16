use std::time::Duration;

use serde_json::Value;

use crate::{AgentCredential, AgentJobStatus};

pub fn validate(
    process_definition: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
) -> Result<String, String> {
    let process: Value = serde_json::from_str(process_definition)
        .map_err(|error| format!("Jira Process Config is invalid: {error}"))?;
    let jira = process
        .pointer("/details/jira")
        .ok_or_else(|| "Process Config details.jira is required.".to_string())?;
    let base_url = jira.get("url").and_then(Value::as_str).unwrap_or("").trim();
    let account = jira
        .get("account")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let issue = job.target.as_deref().unwrap_or("").trim();
    let credential = credentials
        .iter()
        .find(|item| item.reference.eq_ignore_ascii_case(account))
        .ok_or_else(|| format!("Jira credential '{account}' is unavailable in Agent memory."))?;
    let url = format!(
        "{}/rest/api/2/issue/{}?fields=summary,status",
        base_url.trim_end_matches('/'),
        issue
    );
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not initialize Jira HTTP client: {error}"))?
        .get(&url)
        .bearer_auth(&credential.password)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send()
        .map_err(|error| format!("Jira request failed for {issue}: {error}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if status.as_u16() == 401 {
        return Err(
            "Jira authentication failed (HTTP 401). Update the Personal Access Token in KeePassXC."
                .into(),
        );
    }
    if !status.is_success() {
        return Err(format!(
            "Jira API HTTP {}: {}",
            status.as_u16(),
            body.chars().take(1000).collect::<String>()
        ));
    }
    Ok(format!("Jira validation passed for {issue}."))
}
