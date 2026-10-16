use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

use crate::{AgentCredential, AgentJobStatus};

pub fn validate(
    process_definition: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
) -> Result<String, String> {
    let process: Value = serde_json::from_str(process_definition)
        .map_err(|error| format!("CX Process Config is invalid: {error}"))?;
    let cx = process
        .pointer("/details/cx")
        .ok_or_else(|| "Process Config details.cx is required.".to_string())?;
    let base_url = cx.get("url").and_then(Value::as_str).unwrap_or("").trim();
    let account = cx
        .get("account")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let ticket = job.target.as_deref().unwrap_or("").trim();
    if base_url.is_empty() || account.is_empty() || ticket.is_empty() {
        return Err(
            "CX validation requires details.cx.url, details.cx.account and a request number."
                .into(),
        );
    }
    let credential = credentials
        .iter()
        .find(|item| item.reference.eq_ignore_ascii_case(account))
        .ok_or_else(|| format!("CX credential '{account}' is unavailable in Agent memory."))?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|error| format!("Could not initialize CX HTTP client: {error}"))?
        .get(format!("{}/{}", base_url.trim_end_matches('/'), ticket))
        .basic_auth(&credential.username, Some(&credential.password))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send()
        .map_err(|error| format!("CX request failed for {ticket}: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Could not read CX response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "CX API HTTP {}: {}",
            status.as_u16(),
            body.chars().take(1000).collect::<String>()
        ));
    }
    let ticket_json: Value = serde_json::from_str(&body)
        .map_err(|error| format!("CX returned unreadable JSON: {error}"))?;
    validate_ticket(ticket, &ticket_json)
}

pub(crate) fn validate_ticket(ticket_id: &str, ticket: &Value) -> Result<String, String> {
    let raw_status = field(ticket, "Status")
        .ok_or_else(|| "CX response does not include a Status field.".to_string())?;
    let status = raw_status.split('(').next().unwrap_or(&raw_status).trim();
    if !status.eq_ignore_ascii_case("Scheduled") {
        return Err(format!(
            "CX ticket status must be Scheduled; received {status}."
        ));
    }
    let start_text = field(ticket, "Scheduled Start Date")
        .ok_or_else(|| "CX ticket has no Scheduled Start Date.".to_string())?;
    let end_text = field(ticket, "Scheduled End Date")
        .ok_or_else(|| "CX ticket has no Scheduled End Date.".to_string())?;
    let start = date(&start_text)
        .ok_or_else(|| format!("CX Scheduled Start Date is invalid: {start_text}."))?;
    let end =
        date(&end_text).ok_or_else(|| format!("CX Scheduled End Date is invalid: {end_text}."))?;
    let closed = field(ticket, "Date Closed").unwrap_or_default();
    if !["", "n/a", "null", "none", "-"]
        .iter()
        .any(|value| closed.eq_ignore_ascii_case(value))
    {
        return Err(format!("CX ticket is closed (Date Closed: {closed})."));
    }
    let now = Utc::now();
    if now <= start {
        return Err(format!(
            "CX scheduled start is {start_text}; the change window has not opened yet."
        ));
    }
    if now >= end {
        return Err(format!(
            "CX scheduled end was {end_text}; the change window has closed."
        ));
    }
    Ok(format!(
        "CX validation passed for {ticket_id}. Status: {status}; scheduled window: {start_text} → {end_text}; closed: not set."
    ))
}

fn field(ticket: &Value, name: &str) -> Option<String> {
    let value = ticket.get(name)?;
    if value.is_null() {
        return None;
    }
    let value = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn date(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Some(value.with_timezone(&Utc));
    }
    let normalized = value.trim().strip_suffix(" UTC").unwrap_or(value.trim());
    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%d-%b-%Y %H:%M:%S",
        "%d-%b-%Y %H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(normalized, format) {
            return Some(value.and_utc());
        }
    }
    NaiveDate::parse_from_str(normalized, "%Y-%m-%d")
        .ok()
        .and_then(|value| value.and_hms_opt(0, 0, 0))
        .map(|value| value.and_utc())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;

    #[test]
    fn validates_change_window() {
        let now = Utc::now();
        let ticket = serde_json::json!({
            "Status": "Scheduled (Approved)",
            "Scheduled Start Date": (now - Duration::minutes(5)).to_rfc3339(),
            "Scheduled End Date": (now + Duration::minutes(5)).to_rfc3339(),
            "Date Closed": null
        });
        assert!(
            validate_ticket("REQ-100", &ticket)
                .unwrap()
                .contains("CX validation passed")
        );
    }
}
