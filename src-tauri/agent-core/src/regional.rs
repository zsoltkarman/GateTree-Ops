//! Region lanes are derived from the frozen Plan's inventory host-to-site map.
//! Global jobs form barriers; a lane runs one job at a time, including Thanos.
use crate::{AgentJobSetStatus, AgentJobStatus};
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
pub(crate) struct Schedule {
    pub limit: usize,
    hosts: BTreeMap<String, String>,
}

impl Schedule {
    pub fn parse(plan: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_yaml::from_str(plan).map_err(|e| e.to_string())?;
        let inputs = &value["inputs"];
        let deployment = inputs
            .get("olam")
            .or_else(|| inputs.get("ansible"))
            .or_else(|| inputs.get("container"));
        let Some(deployment) = deployment else {
            return Ok(Self {
                limit: 1,
                ..Self::default()
            });
        };
        let limit = match deployment.get("parallel_regions") {
            None => 1,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=16).contains(n))
                .ok_or("Parallel regions must be a whole number from 1 to 16.")?
                as usize,
        };
        let hosts: BTreeMap<String, String> = deployment
            .pointer("/targets/host_sites")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("Invalid host regions: {e}"))?
            .unwrap_or_default();
        if limit > 1 {
            let targets = &deployment["targets"];
            if targets["job_mode"] != "per_host"
                || targets["inventory_groups"]
                    .as_array()
                    .is_some_and(|groups| !groups.is_empty())
            {
                return Err("Parallel regions requires Separate job per host.".into());
            }
            let sites = targets["sites"]
                .as_array()
                .ok_or("Select deployment regions.")?;
            let selected = targets["hosts"]
                .as_array()
                .filter(|hosts| !hosts.is_empty())
                .ok_or("Select deployment hosts.")?;
            for host in selected {
                let site = host.as_str().and_then(|host| hosts.get(host)).ok_or(
                    "Every host needs a deployment region. Select deployment targets again.",
                )?;
                if !sites
                    .iter()
                    .any(|selected| selected.as_str() == Some(site.as_str()))
                {
                    return Err("Host region is not selected in the Plan.".into());
                }
            }
        }
        Ok(Self { limit, hosts })
    }

    pub fn region<'a>(&'a self, job: &AgentJobStatus) -> Option<&'a str> {
        let scoped = job.driver == "thanos"
            || matches!(job.driver.as_str(), "olam" | "container")
                && matches!(
                    job.action.as_str(),
                    "deploy" | "validate_deploy" | "validate_deployment"
                );
        scoped
            .then(|| self.hosts.get(job.target.as_deref()?))
            .flatten()
            .map(String::as_str)
    }

    pub fn validate_jobs(&self, set: &AgentJobSetStatus) -> Result<(), String> {
        if self.limit <= 1 {
            return Ok(());
        }
        for job in &set.jobs {
            if matches!(job.driver.as_str(), "olam" | "container")
                && matches!(
                    job.action.as_str(),
                    "deploy" | "validate_deploy" | "validate_deployment"
                )
                && self.region(job).is_none()
            {
                return Err(
                    "Parallel regions requires each deployment job to target one mapped host."
                        .into(),
                );
            }
        }
        Ok(())
    }

    // Jobs in other regions do not block a retry. Global barriers still do.
    pub fn preceding_complete(&self, set: &AgentJobSetStatus, index: usize) -> bool {
        let region = self.region(&set.jobs[index]);
        set.jobs[..index].iter().all(|job| {
            done(job)
                || self.limit > 1
                    && region.is_some()
                    && self.region(job).is_some()
                    && self.region(job) != region
        })
    }

    pub fn failed(set: &AgentJobSetStatus) -> Option<&str> {
        set.jobs
            .iter()
            .find(|job| matches!(job.status.as_str(), "failed" | "warning" | "denied"))
            .map(|job| job.status.as_str())
    }

    /// A regional failure only blocks that lane. Unscoped failures remain
    /// Plan-wide barriers (for example Jira/CX prerequisite validation).
    pub fn blocked(&self, set: &AgentJobSetStatus, job: &AgentJobStatus) -> bool {
        let region = self.region(job);
        set.jobs.iter().any(|failed| {
            matches!(failed.status.as_str(), "failed" | "warning" | "denied")
                && (self.region(failed).is_none() || self.region(failed) == region)
        })
    }

    /// Candidate heads of the first unfinished regional block. Failed lanes
    /// release their slots; healthy lanes keep running and admit new regions.
    pub fn candidates(
        &self,
        set: &AgentJobSetStatus,
        admitted: &mut HashSet<String>,
    ) -> Vec<usize> {
        let Some(first) = set.jobs.iter().position(|job| !done(job)) else {
            return vec![];
        };
        if self.region(&set.jobs[first]).is_none() {
            admitted.clear();
            return if Self::failed(set).is_some()
                || !matches!(
                    set.jobs[first].status.as_str(),
                    "queued" | "running" | "awaiting_approval"
                ) {
                vec![]
            } else {
                vec![first]
            };
        }
        let end = (first..set.jobs.len())
            .find(|&i| self.region(&set.jobs[i]).is_none())
            .unwrap_or(set.jobs.len());
        let mut heads = Vec::new();
        let mut seen = HashSet::new();
        for i in first..end {
            let job = &set.jobs[i];
            let region = self.region(job).unwrap();
            if !done(job)
                && seen.insert(region)
                && matches!(
                    job.status.as_str(),
                    "queued" | "running" | "awaiting_approval"
                )
                && !self.blocked(set, job)
            {
                heads.push((region, i));
            }
        }
        admitted.retain(|region| heads.iter().any(|(head, _)| *head == region.as_str()));
        for (region, i) in &heads {
            if matches!(
                set.jobs[*i].status.as_str(),
                "running" | "awaiting_approval"
            ) {
                admitted.insert((*region).into());
            }
        }
        for (region, _) in &heads {
            if admitted.len() >= self.limit {
                break;
            }
            admitted.insert((*region).into());
        }
        heads
            .into_iter()
            .filter_map(|(region, i)| admitted.contains(region).then_some(i))
            .collect()
    }
}

pub(crate) fn done(job: &AgentJobStatus) -> bool {
    matches!(job.status.as_str(), "completed" | "disabled")
}

#[cfg(test)]
mod tests;
