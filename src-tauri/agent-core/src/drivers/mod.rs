mod cx;
mod jira;
mod olam;

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
) -> Option<Result<DriverOutcome, String>> {
    match (job.driver.as_str(), job.action.as_str()) {
        ("jira", "validate") => {
            Some(jira::validate(process_definition, job, credentials).map(DriverOutcome::Completed))
        }
        ("cx", "validate") => {
            Some(cx::validate(process_definition, job, credentials).map(DriverOutcome::Completed))
        }
        ("olam", "deploy") => Some(olam::execute(
            process_definition,
            plan_definition,
            job,
            credentials,
        )),
        _ => None,
    }
}
