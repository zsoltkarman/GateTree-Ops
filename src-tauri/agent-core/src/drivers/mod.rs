mod cx;
pub(crate) mod cx_update;
mod jira;
pub(crate) mod jira_update;
mod olam;
mod report;

use crate::{AgentCredential, AgentJobStatus};

pub enum DriverOutcome {
    Completed(String),
    AwaitingApproval(String),
    Warning(String),
}

pub fn execute(
    process_definition: &str,
    plan_definition: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_started: &dyn Fn(&str, i64),
    on_output: &dyn Fn(&str),
) -> Result<DriverOutcome, String> {
    match (job.driver.as_str(), job.action.as_str()) {
        ("jira", "validate") => {
            jira::validate(process_definition, job, credentials).map(DriverOutcome::Completed)
        }
        ("cx", "validate") => {
            cx::validate(process_definition, job, credentials).map(DriverOutcome::Completed)
        }
        ("jira" | "cx", "update") => {
            Err("Update requires Plan report context and durable checkpoints.".into())
        }
        ("olam", "validate") => {
            olam::validate_endpoint(process_definition, credentials).map(DriverOutcome::Completed)
        }
        ("olam", "validate_deployment" | "deploy" | "validate_deploy") => olam::execute(
            process_definition,
            plan_definition,
            job,
            credentials,
            on_started,
            on_output,
        ),
        _ => Err(format!(
            "Action {}/{} is not implemented. No external operation was performed.",
            job.driver, job.action
        )),
    }
}
