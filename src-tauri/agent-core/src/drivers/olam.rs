use std::{thread, time::Duration};

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

use super::DriverOutcome;
use crate::{AgentCredential, AgentJobStatus};

pub fn execute(
    process_definition: &str,
    plan_definition: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
) -> Result<DriverOutcome, String> {
    let process: Value = serde_json::from_str(process_definition)
        .map_err(|error| format!("OLAM Process Config is invalid: {error}"))?;
    let olam = process
        .pointer("/details/olam")
        .ok_or_else(|| "Process Config details.olam is required.".to_string())?;
    let base_url = required_string(olam, "url")?;
    let api_account = required_string(olam, "account")?;
    let environment = olam
        .get("payload_defaults")
        .ok_or_else(|| "Process Config details.olam.payload_defaults is required.".to_string())
        .and_then(|defaults| required_string(defaults, "ENVIRONMENT"))?;
    let phase = job.phase.as_deref().unwrap_or("validation");
    let template_key = if phase == "deployment" {
        "deployment_template_job"
    } else {
        "validation_template_job"
    };
    let template_id_key = if phase == "deployment" {
        "deployment_template_id"
    } else {
        "validation_template_id"
    };
    let template_name = required_string(olam, template_key)?;
    let template_id = required_id(olam, template_id_key)?;
    let api_credential = credential(credentials, api_account, "OLAM API")?;
    let deployment_account = deployment_account(plan_definition)?;
    let deployment_credential = credential(credentials, &deployment_account, "deployment")?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(
            !olam
                .get("verify_tls_certificate")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
        .build()
        .map_err(|error| format!("Could not initialize OLAM HTTP client: {error}"))?;

    verify_template(
        &client,
        base_url,
        template_id,
        template_name,
        api_credential,
    )?;
    let payload = resolved_payload(job, deployment_credential, environment)?;
    let launch_url = format!(
        "{}/api/v2/job_templates/{template_id}/launch/",
        base_url.trim_end_matches('/')
    );
    let launch: Value = checked(
        client
            .post(&launch_url)
            .basic_auth(&api_credential.username, Some(&api_credential.password))
            .json(&json!({ "extra_vars": payload }))
            .send()
            .map_err(|error| {
                format!("Could not launch OLAM template '{template_name}': {error}")
            })?,
        &format!("launch OLAM template '{template_name}'"),
    )?
    .json()
    .map_err(|error| format!("OLAM launch response is invalid: {error}"))?;
    let job_id = launch
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "OLAM did not return an Ansible job ID.".to_string())?;
    let output = wait_for_job(
        &client,
        base_url,
        job_id,
        api_credential,
        phase == "deployment",
    )?;
    let log = format!(
        "OLAM {phase} template '{template_name}' (ID {template_id}) completed successfully as Ansible job {job_id}.\n{output}"
    );
    if output_indicates_no_hosts(&output) {
        return Ok(DriverOutcome::Warning(format!(
            "{log}\nWARNING: OLAM reported that no hosts matched. The Queue has stopped and no following job will start."
        )));
    }
    if phase == "deployment" {
        Ok(DriverOutcome::Completed(log))
    } else {
        Ok(DriverOutcome::AwaitingApproval(format!(
            "{log}\nValidation passed. Review the output and approve this Queue job to start deployment."
        )))
    }
}

fn output_indicates_no_hosts(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    output.contains("skipping: no hosts matched") || output.contains("no hosts matched")
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Process Config details.olam.{key} is required."))
}

fn required_id(value: &Value, key: &str) -> Result<i64, String> {
    let id = value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    });
    id.filter(|id| *id > 0)
        .ok_or_else(|| format!("Process Config details.olam.{key} must be a positive number."))
}

fn credential<'a>(
    credentials: &'a [AgentCredential],
    reference: &str,
    purpose: &str,
) -> Result<&'a AgentCredential, String> {
    credentials
        .iter()
        .find(|item| item.reference.eq_ignore_ascii_case(reference))
        .ok_or_else(|| {
            format!("{purpose} credential '{reference}' is unavailable in Agent memory.")
        })
}

fn deployment_account(plan_definition: &str) -> Result<String, String> {
    let plan: serde_yaml::Value = serde_yaml::from_str(plan_definition)
        .map_err(|error| format!("Deployment Plan YAML is invalid: {error}"))?;
    plan.get("inputs")
        .and_then(|inputs| inputs.get("olam"))
        .and_then(|olam| olam.get("account"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Deployment Plan inputs.olam.account is required.".into())
}

fn resolved_payload(
    job: &AgentJobStatus,
    credential: &AgentCredential,
    environment: &str,
) -> Result<String, String> {
    let mut payload: Value = serde_json::from_str(job.payload.as_deref().unwrap_or("{}"))
        .map_err(|error| format!("OLAM job payload is invalid: {error}"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "OLAM job payload must be a JSON object.".to_string())?;
    object.insert("ansible_user".into(), json!(credential.username));
    object.insert("ansible_password".into(), json!(credential.password));
    object.insert("ansible_become_pass".into(), json!(credential.password));
    object.insert("ENVIRONMENT".into(), json!(environment));
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

fn verify_template(
    client: &Client,
    base_url: &str,
    id: i64,
    name: &str,
    credential: &AgentCredential,
) -> Result<(), String> {
    let url = format!(
        "{}/api/v2/job_templates/{id}/",
        base_url.trim_end_matches('/')
    );
    let value: Value = checked(
        client
            .get(&url)
            .basic_auth(&credential.username, Some(&credential.password))
            .send()
            .map_err(|error| format!("Could not verify OLAM template ID {id}: {error}"))?,
        &format!("verify OLAM template ID {id}"),
    )?
    .json()
    .map_err(|error| format!("OLAM template response is invalid: {error}"))?;
    let actual_name = value.get("name").and_then(Value::as_str).unwrap_or("");
    if !actual_name.eq_ignore_ascii_case(name) {
        return Err(format!(
            "OLAM template ID {id} is '{actual_name}', but Process Config requires '{name}'. No job was started."
        ));
    }
    Ok(())
}

fn wait_for_job(
    client: &Client,
    base_url: &str,
    job_id: i64,
    credential: &AgentCredential,
    deployment: bool,
) -> Result<String, String> {
    let attempts = if deployment { 21_600 } else { 300 };
    let url = format!("{}/api/v2/jobs/{job_id}/", base_url.trim_end_matches('/'));
    for _ in 0..attempts {
        let value: Value = checked(
            client
                .get(&url)
                .basic_auth(&credential.username, Some(&credential.password))
                .send()
                .map_err(|error| format!("Could not poll OLAM job {job_id}: {error}"))?,
            &format!("poll OLAM job {job_id}"),
        )?
        .json()
        .map_err(|error| format!("OLAM job {job_id} response is invalid: {error}"))?;
        match value.get("status").and_then(Value::as_str).unwrap_or("") {
            "successful" => return job_output(client, base_url, job_id, credential),
            "failed" | "error" | "canceled" => {
                let output = job_output(client, base_url, job_id, credential).unwrap_or_default();
                return Err(format!("OLAM job {job_id} failed.\n{output}"));
            }
            _ => thread::sleep(Duration::from_secs(2)),
        }
    }
    Err(format!(
        "OLAM job {job_id} did not finish before the timeout."
    ))
}

fn job_output(
    client: &Client,
    base_url: &str,
    job_id: i64,
    credential: &AgentCredential,
) -> Result<String, String> {
    let url = format!(
        "{}/api/v2/jobs/{job_id}/stdout/?format=txt",
        base_url.trim_end_matches('/')
    );
    let response = checked(
        client
            .get(&url)
            .basic_auth(&credential.username, Some(&credential.password))
            .header("Accept", "text/plain, */*")
            .send()
            .map_err(|error| format!("Could not read OLAM job {job_id} output: {error}"))?,
        &format!("read OLAM job {job_id} output"),
    )?;
    let text = response.text().map_err(|error| error.to_string())?;
    Ok(text.chars().take(512_000).collect())
}

fn checked(response: Response, operation: &str) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    Err(format!(
        "Could not {operation}: HTTP {}: {}",
        status.as_u16(),
        body.chars().take(1000).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::output_indicates_no_hosts;

    #[test]
    fn treats_no_matching_hosts_as_a_warning() {
        assert!(output_indicates_no_hosts(
            "[WARNING]: Could not match supplied host pattern\nPLAY [Ping Servers]\nskipping: no hosts matched"
        ));
        assert!(!output_indicates_no_hosts(
            "PLAY [Ping Servers]\nok: [host-1]"
        ));
    }
}
