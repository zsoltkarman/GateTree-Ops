//! Refresh query text without restarting deployments or changing execution state.
use rusqlite::params;
use serde_json::{Value, json};

use super::{
    queue_service::{QueueError, resolve_thanos_query, thanos_bindings},
    validation_queries,
};
use crate::{agent_ipc, modules::workspace::WorkspaceState};

pub(super) fn refresh(state: &WorkspaceState, notify_agent: bool) -> Result<(), QueueError> {
    let updates = state.with_connection(|db| {
        let queries = validation_queries::load(db)?;
        let rows = db.prepare(
            "SELECT j.id,j.sequence,j.payload_json,p.definition_yaml,COALESCE(p.process_definition_json,c.definition_json)
             FROM deployment_jobs j JOIN deployment_plans p ON p.id=j.plan_id
             LEFT JOIN deployment_processes c ON c.name=p.process_name COLLATE NOCASE
             WHERE j.driver='thanos' AND j.status<>'completed' AND p.status<>'completed'
             AND NOT EXISTS(SELECT 1 FROM workspace_metadata m WHERE m.key='deployment_plan_archived:' || p.id AND m.value='1')"
        )?.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,
            row.get::<_,Option<String>>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?.unwrap_or_default())))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let tx = db.unchecked_transaction()?;
        let mut updates = Vec::new();
        for (id, sequence, payload, definition, process) in rows {
            let payload = current_payload(payload.as_deref(), sequence, &definition, &process, &queries);
            let payload = payload.to_string();
            tx.execute("UPDATE deployment_jobs SET payload_json=?2 WHERE id=?1",params![id,payload])?;
            updates.push(agent_ipc::ThanosQueryUpdate { job_id:id, payload });
        }
        tx.commit()?;
        Ok(updates)
    })?;
    // An offline Agent receives the refreshed payload at Plan submission. An
    // online Agent also updates queued jobs and the next poll of running checks.
    if notify_agent && !updates.is_empty() && agent_ipc::status()?.running {
        agent_ipc::refresh_thanos_queries(updates)?;
    }
    Ok(())
}

pub(super) fn submission_queries(
    db: &rusqlite::Connection,
    jobs: &mut [agent_ipc::AgentJobSubmission],
    definition: &str,
    process: &str,
) -> rusqlite::Result<()> {
    let queries = validation_queries::load(db)?;
    for job in jobs.iter_mut().filter(|job| job.driver == "thanos") {
        job.payload = Some(
            current_payload(
                job.payload.as_deref(),
                job.sequence,
                definition,
                process,
                &queries,
            )
            .to_string(),
        );
    }
    Ok(())
}

fn current_payload(
    payload: Option<&str>,
    sequence: i64,
    definition: &str,
    process: &str,
    queries: &[validation_queries::ValidationQuery],
) -> Value {
    let mut payload: Value = serde_json::from_str(payload.unwrap_or("{}")).unwrap_or(json!({}));
    if !payload.is_object() {
        payload = json!({});
    }
    if payload.get("query_context").is_none() {
        match thanos_bindings(definition, process).and_then(|bindings| {
            bindings.into_iter().find(|(seq,_)| *seq == sequence)
                .ok_or_else(|| QueueError::Invalid("Cannot identify the original Thanos step. Edit the Plan and select its saved query.".into()))
        }) {
            Ok((_, binding)) => {
                let binding: Value = serde_json::from_str(&binding).expect("generated query payload");
                if binding["query"] != payload["query"] {
                    payload["query_error"] = json!("The original Thanos job no longer matches its Plan. Edit the Plan and select its saved query.");
                    return payload;
                }
                payload = binding;
            },
            Err(error) => { payload["query_error"] = json!(error.to_string()); }
        }
    }
    refresh_payload(&mut payload, queries);
    payload
}

fn refresh_payload(payload: &mut Value, queries: &[validation_queries::ValidationQuery]) {
    let result = (|| -> Result<(), String> {
        let source = if let Some(id) = payload["validation_query_id"].as_str() {
            queries.iter().find(|query| query.id == id)
        } else {
            // Upgrade old Plans by an exact, unique library match. Never guess
            // from a role/name when several saved checks could apply.
            let matches: Vec<_> = queries.iter().filter(|query|
                Some(query.query.trim()) == payload["query_template"].as_str().map(str::trim)).collect();
            if matches.len() == 1 { Some(matches[0]) } else {
                let names: Vec<_> = queries.iter().filter(|query|
                    payload["query_name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(&query.name))).collect();
                if names.len() == 1 { Some(names[0]) } else { None }
            }
        }.ok_or("Saved Thanos query is missing or cannot be identified. Edit the Plan and select a query from Config → Thanos Queries.")?;
        payload["validation_query_id"] = json!(source.id);
        let context = &payload["query_context"];
        if context.is_null() {
            return Err("Thanos query deployment context is missing. Edit the Plan and select its saved query.".into());
        }
        let host = context["host"].as_str();
        if host.is_some() && !source.query.contains("${DEPLOY_HOST_REGEX}") {
            return Err("Per-host Thanos validation requires ${DEPLOY_HOST_REGEX} in the current saved query.".into());
        }
        if source.query.contains("${DEPLOY_IMAGE_VERSION}")
            && (host.is_none() || context["preceding_deploy"] != true)
        {
            return Err(
                "DEPLOY_IMAGE_VERSION requires a preceding Deploy step and Separate job per host."
                    .into(),
            );
        }
        let query = resolve_thanos_query(
            &source.query,
            context["region"].as_str(),
            context["expected_version"].as_str(),
            host,
        )
        .map_err(|error| error.to_string())?;
        payload["query"] = json!(query);
        payload["query_revision"] = json!(source.revision);
        Ok(())
    })();
    match result {
        Ok(()) => {
            payload
                .as_object_mut()
                .expect("query payload object")
                .remove("query_error");
        }
        Err(error) => {
            payload["query_error"] = json!(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;
    use validation_queries::ValidationQuery;

    fn saved(id: &str, query: &str) -> ValidationQuery {
        ValidationQuery {
            id: id.into(),
            name: "Exporter".into(),
            query: query.into(),
            revision: 2,
        }
    }
    fn binding() -> Value {
        json!({"query":"old", "query_template":"old", "validation_query_id":"saved",
            "query_context":{"region":"phx_icstg","host":"redis1.example.test",
            "expected_version":"1.97.0","preceding_deploy":true}})
    }

    #[test]
    fn legacy_job_recovers_source_and_context_from_original_plan() {
        let original = r#"metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}"}"#;
        let plan = json!({"plan_name":"Upgrade","process":"Deploy","inputs":{"olam":{
            "strategy":"linear","account":"deploy","targets":{"roles":["exporter"],"sites":["phx"],
            "hosts":["redis1.example.test"],"job_mode":"per_host"},
            "steps":[{"name":"Upgrade"},{"name":"Exporter","type":"thanos_query","query":original}]
        }}})
        .to_string();
        let process = json!({"steps":[{"name":"Deploy","driver":"olam","action":"deploy","workflow":["approve","deploy"]}],
            "details":{"olam":{"url":"https://olam.example.test","account":"api","payload_defaults":{"ENVIRONMENT":"staging"}},
            "thanos":{"url":"https://thanos.example.test","timeout_seconds":300,"interval_seconds":30,"region_map":{"phx":"phx_icstg"}}}}).to_string();
        let bindings = thanos_bindings(&plan, &process).unwrap();
        let (sequence, full) = &bindings[0];
        let full: Value = serde_json::from_str(full).unwrap();
        let legacy = json!({"query":full["query"]}).to_string();
        let linked = current_payload(
            Some(&legacy),
            *sequence,
            &plan,
            &process,
            &[saved("saved", original)],
        );
        assert_eq!(linked["validation_query_id"], "saved");
        assert_eq!(linked["query_context"]["region"], "phx_icstg");
        assert_eq!(linked["query_context"]["host"], "redis1.example.test");
        assert!(linked.get("query_error").is_none());
        let modified = saved("saved", r#"new_metric{instance=~"${DEPLOY_HOST_REGEX}"}"#);
        let latest = current_payload(
            Some(&linked.to_string()),
            *sequence,
            &plan,
            &process,
            &[modified],
        );
        assert!(latest["query"].as_str().unwrap().starts_with("new_metric"));
        // Query refresh must not guess if an older generator used another order.
        let wrong = current_payload(
            Some(r#"{"query":"different"}"#),
            *sequence,
            &plan,
            &process,
            &[],
        );
        assert!(
            wrong["query_error"]
                .as_str()
                .unwrap()
                .contains("no longer matches")
        );
    }

    #[test]
    fn current_config_replaces_snapshot_and_keeps_deployment_context() {
        let mut payload = binding();
        let current = saved(
            "saved",
            r#"new_metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${DEPLOY_IMAGE_VERSION}",exporter="${EXPECTED_VERSION}"}"#,
        );
        refresh_payload(&mut payload, &[current]);
        let query = payload["query"].as_str().unwrap();
        assert!(query.starts_with("new_metric{"));
        assert!(query.contains("phx_icstg"));
        assert!(query.contains(r"redis1\\.example\\.test"));
        assert!(
            query.contains("${DEPLOY_IMAGE_VERSION}"),
            "Agent binds the completed deployment's version"
        );
        assert!(query.contains("1.97.0"));
        assert_eq!(payload["query_revision"], 2);
        assert!(payload.get("query_error").is_none());
    }

    #[test]
    fn deleted_sources_and_incompatible_edits_fail_without_stale_fallback() {
        let mut payload = binding();
        refresh_payload(&mut payload, &[]);
        assert!(payload["query_error"].as_str().unwrap().contains("missing"));
        // A new query with identical text/name cannot replace a deleted ID.
        refresh_payload(&mut payload, &[saved("replacement", "old")]);
        assert!(payload.get("query_error").is_some());
        refresh_payload(&mut payload, &[saved("saved", "vector(1)")]);
        assert!(
            payload["query_error"]
                .as_str()
                .unwrap()
                .contains("DEPLOY_HOST_REGEX")
        );
        payload["query_context"]["host"] = Value::Null;
        refresh_payload(&mut payload, &[saved("saved", "vector(1)")]);
        assert_eq!(payload["query"], "vector(1)");
        assert!(payload.get("query_error").is_none());
    }

    #[test]
    fn legacy_binding_is_unique_and_survives_later_template_edits() {
        let mut payload = binding();
        payload
            .as_object_mut()
            .unwrap()
            .remove("validation_query_id");
        payload["query_context"]["host"] = Value::Null;
        refresh_payload(&mut payload, &[saved("one", "old"), saved("two", "old")]);
        assert!(payload.get("query_error").is_some());
        refresh_payload(&mut payload, &[saved("one", "old")]);
        assert_eq!(payload["validation_query_id"], "one");
        refresh_payload(&mut payload, &[saved("one", "vector(1)")]);
        assert_eq!(payload["query"], "vector(1)");
    }

    #[test]
    fn refresh_updates_failed_queries_preserves_completed_jobs_and_survives_reopen() {
        let directory =
            std::env::temp_dir().join(format!("gt-live-queries-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "test password",
        )
        .unwrap();
        let mut payload = binding();
        payload["query_context"]["host"] = Value::Null;
        state.with_connection(|db| {
            db.execute("INSERT INTO deployment_processes(id,name,definition_json) VALUES('process','Deploy','{}')",[])?;
            db.execute("INSERT INTO deployment_plans(id,plan_number,name,process_name,definition_yaml,process_definition_json,status) VALUES('plan',1,'Upgrade','Deploy','inputs: {}','{}','failed')",[])?;
            db.execute("INSERT INTO deployment_validation_queries(id,name,role,query) VALUES('saved','Exporter','','vector(1)')",[])?;
            for (id,sequence,driver,status) in [("deploy",1,"olam","completed"),("check",2,"thanos","failed"),("done",3,"thanos","completed")] {
                db.execute("INSERT INTO deployment_jobs(id,plan_id,sequence,name,driver,action,payload_json,status,log_output) VALUES(?1,'plan',?2,'Step',?3,'query',?4,?5,'original evidence')",
                    params![id,sequence,driver,payload.to_string(),status])?;
            }
            Ok(())
        }).unwrap();
        refresh(&state, false).unwrap();
        state.with_connection(|db| {
            let mut rows = db.prepare("SELECT id,payload_json,status,log_output FROM deployment_jobs ORDER BY sequence")?;
            let rows = rows.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(rows[0].1,payload.to_string());
            assert_eq!(rows[0].2,"completed");
            let updated: Value = serde_json::from_str(&rows[1].1).unwrap();
            assert_eq!(updated["query"],"vector(1)");
            assert_eq!(rows[1].2,"failed", "refresh must not restart the job or deploy");
            assert_eq!(rows[1].3,"original evidence");
            assert_eq!(rows[2].1,payload.to_string());
            db.execute("UPDATE deployment_validation_queries SET query='vector(0)',revision=revision+1 WHERE id='saved'",[])?;
            Ok(())
        }).unwrap();
        state.lock().unwrap();
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "test password",
        )
        .unwrap();
        refresh(&state, false).unwrap();
        state
            .with_connection(|db| {
                let query: String = db.query_row(
                    "SELECT payload_json FROM deployment_jobs WHERE id='check'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(
                    serde_json::from_str::<Value>(&query).unwrap()["query"],
                    "vector(0)"
                );
                Ok(())
            })
            .unwrap();
        state.lock().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
