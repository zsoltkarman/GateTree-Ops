use crate::{AgentJobSetStatus, AgentJobStatus};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Cursor, Write};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    scope: String,
    #[serde(default)]
    comment_template: Option<String>,
    #[serde(default)]
    deployment_template: Option<String>,
    #[serde(default)]
    attachments: Vec<Attachment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    source: String,
    filename_template: String,
    #[serde(default)]
    compression: Compression,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Compression {
    #[default]
    None,
    Zip,
}

const FIELDS: &[&str] = &[
    "issue",
    "request",
    "plan_name",
    "plan_number",
    "role",
    "target",
    "ip",
    "datetime",
    "status",
    "job_id",
    "validation_job_id",
    "step",
    "job_url",
    "deployments",
];

fn render(template: &str, values: &BTreeMap<String, String>) -> Result<String, String> {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or("Unclosed Deployment template placeholder.")?;
        let name = after[..end].trim();
        if !FIELDS.contains(&name) {
            return Err(format!(
                "Unknown Deployment template placeholder '{{{{{name}}}}}'."
            ));
        }
        rendered.push_str(values.get(name).map(String::as_str).unwrap_or(""));
        rest = &after[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

fn contains_placeholder(template: &str, name: &str) -> bool {
    template.split("{{").skip(1).any(|part| {
        part.split_once("}}")
            .is_some_and(|(field, _)| field.trim() == name)
    })
}

pub fn validate_report(value: &Value) -> Result<(), String> {
    let report: Report = serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid Deployment update report: {e}"))?;
    if !matches!(report.scope.as_str(), "deployment" | "plan") {
        return Err("Deployment report.scope must be 'deployment' or 'plan'.".into());
    }
    if report.comment_template.is_none() && report.attachments.is_empty() {
        return Err(
            "Deployment update report must configure a comment_template or attachments.".into(),
        );
    }
    let empty = BTreeMap::new();
    for template in report
        .comment_template
        .iter()
        .chain(report.deployment_template.iter())
        .chain(report.attachments.iter().map(|a| &a.filename_template))
    {
        if template.trim().is_empty() {
            return Err("Deployment report templates must not be empty.".into());
        }
        render(template, &empty)?;
        if contains_placeholder(template, "deployments")
            && (report.scope != "plan" || report.deployment_template.is_none())
        {
            return Err("{{deployments}} requires scope 'plan' and a deployment_template.".into());
        }
    }
    if report
        .deployment_template
        .as_deref()
        .is_some_and(|t| contains_placeholder(t, "deployments"))
    {
        return Err("deployment_template cannot contain {{deployments}}.".into());
    }
    for attachment in &report.attachments {
        if attachment.compression == Compression::Zip
            && !attachment
                .filename_template
                .to_ascii_lowercase()
                .ends_with(".zip")
        {
            return Err("ZIP attachment filename_template must end in .zip.".into());
        }
        if !(matches!(attachment.source.as_str(), "deployment_log" | "job_log")
            && report.scope == "deployment"
            || attachment.source == "plan_log" && report.scope == "plan")
        {
            return Err("Deployment attachments require deployment_log/job_log for scope 'deployment', or plan_log for scope 'plan'.".into());
        }
    }
    Ok(())
}

pub(super) struct PreparedReport {
    pub(super) key: String,
    pub(super) comment: Option<String>,
    pub(super) attachments: Vec<PreparedAttachment>,
}

pub(super) struct PreparedAttachment {
    pub(super) filename: String,
    pub(super) bytes: Vec<u8>,
    pub(super) mime: &'static str,
}

fn ip_from_log(log: &str) -> String {
    for (offset, _) in log.match_indices("\"msg\"") {
        let after = log[offset + 5..].trim_start();
        let Some(after) = after.strip_prefix(':') else {
            continue;
        };
        let Some(Ok(value)) = serde_json::Deserializer::from_str(after)
            .into_iter::<Value>()
            .next()
        else {
            continue;
        };
        let message = match value {
            Value::String(s) => s,
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        for line in message.replace("\\n", "\n").lines() {
            if let Some(ip) = line.trim().strip_prefix("Ansible ssh host:") {
                if !ip.trim().is_empty() {
                    return ip.trim().into();
                }
            }
        }
    }
    String::new()
}

fn values(
    set: &AgentJobSetStatus,
    issue: &str,
    deployment: Option<&AgentJobStatus>,
    driver: &str,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([
        ("issue".into(), issue.into()),
        (
            "request".into(),
            if driver == "cx" {
                issue.into()
            } else {
                String::new()
            },
        ),
        ("plan_name".into(), set.name.clone()),
        ("plan_number".into(), set.plan_number.to_string()),
        ("status".into(), "completed".into()),
        ("datetime".into(), chrono::Utc::now().to_rfc3339()),
    ]);
    if let Some(job) = deployment {
        let payload: Value =
            serde_json::from_str(job.payload.as_deref().unwrap_or("{}")).unwrap_or(Value::Null);
        values.extend([
            (
                "role".into(),
                payload["playbook_name"].as_str().unwrap_or("").into(),
            ),
            ("target".into(), job.target.clone().unwrap_or_default()),
            ("ip".into(), ip_from_log(&job.log)),
            (
                "datetime".into(),
                job.finished_at.clone().unwrap_or_else(|| "unknown".into()),
            ),
            ("status".into(), job.status.clone()),
            (
                "job_id".into(),
                job.deployment_job_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
            ),
            (
                "validation_job_id".into(),
                job.validation_job_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
            ),
            ("step".into(), job.name.clone()),
        ]);
        let process: Value = serde_json::from_str(&set.process_definition).unwrap_or(Value::Null);
        let url = process
            .pointer("/details/olam/url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches('/');
        values.insert(
            "job_url".into(),
            job.deployment_job_id
                .filter(|_| job.driver == "olam" && !url.is_empty())
                .map(|id| format!("{url}/#/jobs/playbook/{id}/output"))
                .unwrap_or_default(),
        );
    }
    values
}

fn filename(value: String) -> Result<String, String> {
    let name: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(180)
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        return Err("Deployment attachment filename template produced an empty filename.".into());
    }
    Ok(name.into())
}

pub(super) fn prepare_attachment(
    filename: String,
    log: String,
    compression: &Compression,
) -> Result<PreparedAttachment, String> {
    let (bytes, mime) = match compression {
        Compression::None => (log.into_bytes(), "text/plain; charset=utf-8"),
        Compression::Zip => {
            let stem = filename
                .get(..filename.len().saturating_sub(4))
                .filter(|s| !s.is_empty())
                .ok_or("ZIP attachment requires a filename before .zip.")?;
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer
                .start_file(format!("{stem}.log"), options)
                .map_err(|e| format!("Could not create ZIP log entry: {e}"))?;
            writer
                .write_all(log.as_bytes())
                .map_err(|e| format!("Could not compress log: {e}"))?;
            let data = writer
                .finish()
                .map_err(|e| format!("Could not finish ZIP attachment: {e}"))?
                .into_inner();
            (data, "application/zip")
        }
    };
    Ok(PreparedAttachment {
        filename,
        bytes,
        mime,
    })
}

pub(super) fn prepare(
    set: &AgentJobSetStatus,
    current: &AgentJobStatus,
    report: &Report,
) -> Result<Vec<PreparedReport>, String> {
    let issue = current
        .target
        .as_deref()
        .ok_or("Deployment update requires an issue key.")?;
    let preceding: Vec<_> = set
        .jobs
        .iter()
        .filter(|job| job.sequence < current.sequence)
        .collect();
    let deployments: Vec<_> = preceding
        .iter()
        .copied()
        .filter(|job| {
            matches!(job.driver.as_str(), "olam" | "container")
                && matches!(job.action.as_str(), "deploy" | "validate_deploy")
                && job.status == "completed"
                && job.deployment_job_id.is_some()
        })
        .collect();
    if deployments.is_empty() {
        return Err("No completed preceding deployments are available for this Deployment update. Disabled jobs are excluded.".into());
    }
    let groups: Vec<Option<&AgentJobStatus>> = if report.scope == "deployment" {
        deployments.iter().map(|job| Some(*job)).collect()
    } else {
        vec![None]
    };
    groups
        .into_iter()
        .map(|deployment| {
            let mut data = values(set, issue, deployment, &current.driver);
            if let Some(template) = &report.deployment_template {
                let summary = deployments
                    .iter()
                    .map(|job| render(template, &values(set, issue, Some(job), &current.driver)))
                    .collect::<Result<Vec<_>, _>>()?
                    .join("\n");
                data.insert("deployments".into(), summary);
            }
            let comment = report
                .comment_template
                .as_ref()
                .map(|t| render(t, &data))
                .transpose()?;
            if comment.as_deref().is_some_and(|c| c.trim().is_empty()) {
                return Err("Deployment comment template produced an empty comment.".into());
            }
            let attachments = report
                .attachments
                .iter()
                .map(|a| {
                    let name = filename(render(&a.filename_template, &data)?)?;
                    let log = match a.source.as_str() {
                        "plan_log" => preceding
                            .iter()
                            .filter(|job| job.status != "disabled" && !job.log.is_empty())
                            .map(|job| format!("[{}] {}\n{}", job.sequence, job.name, job.log))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        "deployment_log" => {
                            let deployment = deployment.ok_or("Missing deployment for log attachment.")?;
                            let log = &deployment.log;
                            let marker = format!("Starting {} deployment for ", deployment.driver.to_uppercase());
                            log.rfind(&marker)
                                .map(|i| log[i..].to_owned())
                                .unwrap_or_else(|| log.clone())
                        }
                        _ => deployment
                            .ok_or("Missing deployment for log attachment.")?
                            .log
                            .clone(),
                    };
                    if log.trim().is_empty() {
                        return Err(
                            "No log output is available for the configured Deployment attachment."
                                .into(),
                        );
                    }
                    prepare_attachment(name, log, &a.compression)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(PreparedReport {
                key: deployment
                    .map(|j| j.id.clone())
                    .unwrap_or_else(|| "plan".into()),
                comment,
                attachments,
            })
        })
        .collect()
}

pub(super) fn prepare_reports(
    set: &AgentJobSetStatus,
    job: &AgentJobStatus,
) -> Result<Vec<PreparedReport>, String> {
    let value: Value =
        serde_json::from_str(job.payload.as_deref().ok_or("Update report is required.")?)
            .map_err(|e| format!("Invalid update report: {e}"))?;
    validate_report(&value)?;
    let report: Report = serde_json::from_value(value).map_err(|e| e.to_string())?;
    prepare(set, job, &report)
}

#[cfg(test)]
mod container_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn container_reports_include_completed_jobs_without_fabricating_olam_links() {
        let report=json!({"scope":"deployment","comment_template":"{{job_id}}|{{job_url}}","attachments":[{"source":"deployment_log","filename_template":"{{job_id}}.log"}]});
        let (mut set,current,_)=super::super::jira_update::tests::context("https://jira.test",report.clone());
        for job in &mut set.jobs[..3] {job.driver="container".into();job.log=job.log.replace("Starting OLAM deployment for ","Starting CONTAINER deployment for ");}
        let reports=prepare(&set,&current,&serde_json::from_value(report).unwrap()).unwrap();
        assert_eq!(reports.len(),2);assert_eq!(reports[0].comment.as_deref(),Some("200|"));
        let attachment=String::from_utf8(reports[0].attachments[0].bytes.clone()).unwrap();
        assert!(attachment.contains("deployment-output-0"));assert!(!attachment.contains("Ansible ssh host"));
    }
}
