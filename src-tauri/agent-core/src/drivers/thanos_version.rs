//! Resolve image versions from the matching deployment's validation evidence.
//! The local checkout may have changed since the Plan was created.
use crate::{AgentJobSetStatus, AgentJobStatus};
use serde_json::Value;
use std::collections::BTreeSet;

const TOKEN: &str = "${DEPLOY_IMAGE_VERSION}";

pub(crate) fn resolve(set: &AgentJobSetStatus, job: &AgentJobStatus) -> Result<AgentJobStatus, String> {
    let mut payload: Value = serde_json::from_str(job.payload.as_deref().unwrap_or("{}"))
        .map_err(|_| "Invalid Thanos query payload.")?;
    let query = payload["query"].as_str().ok_or("Missing Thanos query.")?;
    if !query.contains(TOKEN) { return Ok(job.clone()); }
    let index = set.jobs.iter().position(|candidate| candidate.id == job.id)
        .ok_or("Thanos job is missing from its Plan.")?;
    let target = job.target.as_deref().filter(|host| !host.is_empty())
        .ok_or("Deployment image version requires a single deployed host.")?;
    let deployment = set.jobs[..index].iter().rev().find(|candidate| {
        matches!(candidate.driver.as_str(), "olam" | "container")
            && matches!(candidate.action.as_str(), "deploy" | "validate_deploy")
            && candidate.target.as_deref() == Some(target)
    }).ok_or("No preceding deployment for this Thanos host.")?;
    if deployment.status != "completed" {
        return Err("The matching deployment must complete before resolving its image version.".into());
    }
    let vars: Value = serde_json::from_str(deployment.payload.as_deref().unwrap_or("{}"))
        .map_err(|_| "Invalid deployment payload.")?;
    let role = vars["playbook_name"].as_str().filter(|s| !s.is_empty())
        .ok_or("The deployment has no role name.")?;
    let version = version_from_log(&deployment.log, target, role)?;
    let encoded = serde_json::to_string(&version).map_err(|e| e.to_string())?;
    // Only interpolate inside PromQL double-quoted strings. Also validate at
    // execution time, since Agent submissions need not come from the Planner.
    let mut quote = None;
    let mut escaped = false;
    let mut resolved = String::new();
    let mut remaining = query;
    while !remaining.is_empty() {
        if remaining.starts_with(TOKEN) {
            if quote != Some('"') || escaped { return Err("Image version placeholder must be inside a double-quoted label value.".into()); }
            resolved.push_str(&encoded[1..encoded.len()-1]);
            remaining = &remaining[TOKEN.len()..];
        } else {
            let c = remaining.chars().next().unwrap();
            remaining = &remaining[c.len_utf8()..];
            resolved.push(c);
            if escaped { escaped = false; }
            else if quote.is_some() && c == '\\' && quote != Some('`') { escaped = true; }
            else if quote == Some(c) { quote = None; }
            else if quote.is_none() && matches!(c, '"' | '\'' | '`') { quote = Some(c); }
        }
    }
    if quote.is_some() || resolved.contains("${") || resolved.len() > 32768 {
        return Err("Deployment image version did not produce a complete PromQL query.".into());
    }
    payload["query"] = Value::String(resolved);
    let mut result = job.clone();
    result.payload = Some(payload.to_string());
    Ok(result)
}

fn version_from_log(log: &str, host: &str, role: &str) -> Result<String, String> {
    // Remove terminal colours before reading Ansible task headers.
    let mut text = String::new();
    let mut chars = log.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() { if ('@'..='~').contains(&c) { break; } }
        } else { text.push(c); }
    }
    let mut roles = BTreeSet::new();
    let mut versions = BTreeSet::new();
    for (offset, _) in text.match_indices("\"msg\"") {
        let Some(after) = text[offset+5..].trim_start().strip_prefix(':') else { continue; };
        let Some(Ok(value)) = serde_json::Deserializer::from_str(after).into_iter::<Value>().next() else { continue; };
        let message = match value {
            Value::String(s) => s,
            Value::Array(a) if a.iter().all(Value::is_string) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
            _ => continue,
        }.replace("\\n", "\n").replace("\\r", "\r");
        let fields: Vec<(String,String)> = {
            let mut fields: Vec<(String,String)> = Vec::new();
            for line in message.lines().map(str::trim) {
                if let Some((key, value)) = line.split_once(':') {
                    fields.push((key.trim().into(), value.trim().into()));
                } else if !line.is_empty() && let Some((_, value)) = fields.last_mut() && value.is_empty() {
                    *value = line.into();
                }
            }
            fields
        };
        let header = text[..offset].lines().rev().find_map(|line| {
            let line = line.trim();
            let tail = line.strip_prefix("ok: [").or_else(|| line.strip_prefix("changed: ["))?;
            tail.split_once("] =>").map(|(host,_)| host)
        });
        let declared_host = fields.iter().find(|(key,_)| key == "Ansible hostname").map(|(_,v)| v.as_str());
        if header.or(declared_host) != Some(host) || declared_host.is_some_and(|v| v != host) { continue; }
        for (key,value) in fields {
            match key.as_str() {
                "Role name" => { roles.insert(value); },
                "IMAGE_VERSION" => { versions.insert(value); },
                _ => {},
            }
        }
    }
    if roles.len() != 1 || !roles.contains(role) || versions.len() != 1 {
        return Err("Cannot resolve an unambiguous role IMAGE_VERSION from this host's deployment validation log. Run deployment validation again.".into());
    }
    let version = versions.into_iter().next().unwrap();
    if version.is_empty() || version.len() > 256 || version.contains(['{','}','\n','\r']) || matches!(version.as_str(), "n/a" | "null" | "None") {
        return Err("The deployment's role IMAGE_VERSION is empty or unresolved.".into());
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binds_nearest_same_host_completed_deployment_and_escapes_version() {
        let job = |id: &str, driver: &str, target: &str, payload: Value, log: &str| -> AgentJobStatus {
            serde_json::from_value(serde_json::json!({"id":id,"sequence":1,"name":id,"driver":driver,
                "action":if driver=="thanos" {"query"} else {"deploy"},"target":target,"details":"",
                "payload":payload.to_string(),"status":"completed","log":log})).unwrap()
        };
        let deployment = job("deploy","olam","host",serde_json::json!({"playbook_name":"collector"}),
            "ok: [host] => {\"msg\":[\"Role name: collector\",\"IMAGE_VERSION: 2026.11.2\"]}");
        let other = job("other","olam","elsewhere",serde_json::json!({"playbook_name":"different"}), "");
        let query = job("check","thanos","host",serde_json::json!({"query":"metric{version=\"${DEPLOY_IMAGE_VERSION}\"}"}), "");
        let mut set: AgentJobSetStatus = serde_json::from_value(serde_json::json!({"id":"set","name":"plan",
            "processName":"process","planDefinition":"","processDefinition":"","status":"running",
            "jobs":[deployment,other,query.clone()]})).unwrap();
        let resolved = resolve(&set,&query).unwrap();
        let payload: Value = serde_json::from_str(resolved.payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["query"],"metric{version=\"2026.11.2\"}");
        assert!(query.payload.as_deref().unwrap().contains("${DEPLOY_IMAGE_VERSION}"));
        set.jobs[0].status="failed".into();
        assert!(resolve(&set,&query).is_err());
        set.jobs[0].status="completed".into();
        set.jobs[0].log="ok: [host] => {\"msg\":[\"Role name: collector\",\"IMAGE_VERSION: a\\\"b\\\\c\"]}".into();
        let resolved = resolve(&set,&query).unwrap();
        let payload: Value = serde_json::from_str(resolved.payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["query"],"metric{version=\"a\\\"b\\\\c\"}");
        let mut invalid = query.clone(); invalid.payload=Some(serde_json::json!({"query":"${DEPLOY_IMAGE_VERSION}"}).to_string());
        assert!(resolve(&set,&invalid).is_err());
    }
    #[test]
    fn selects_host_role_and_version_from_actual_preview() {
        let log = "ok: [other] => {\"msg\": [\"Role name: wrong\", \"IMAGE_VERSION: 9\"]}\n\u{1b}[32mok: [host] => {\"msg\": \"Role name: collector\\nAnsible hostname: host\"}\u{1b}[0m\nok: [host] => {\"msg\": [\"IMAGE_VERSION:\",\"2026.11.2\"]}";
        assert_eq!(version_from_log(log,"host","collector").unwrap(),"2026.11.2");
        assert!(version_from_log(log,"host","wrong").is_err());
        assert!(version_from_log(log,"missing","collector").is_err());
        assert!(version_from_log(&format!("{log}\nok: [host] => {{\"msg\": \"IMAGE_VERSION: 1\"}}"),"host","collector").is_err());
        assert!(version_from_log("ok: [host] => {\"msg\":[\"Role name: collector\",\"IMAGE_VERSION: {{ IMAGE_VERSION }}\"]}","host","collector").is_err());
    }
}
