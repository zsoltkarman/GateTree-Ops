use super::{build_api::BuildApi, config::ImageHubConfig, model::Repository};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};

pub trait Database {
    fn with<T>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String>;
}
pub struct WorkspaceDb<'a> {
    pub state: &'a crate::modules::workspace::WorkspaceState,
    pub session: &'a str,
}
impl Database for WorkspaceDb<'_> {
    fn with<T>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        self.state
            .with_session_connection(self.session, operation)
            .map_err(|e| e.to_string())
    }
}
#[derive(Default)]
pub struct ArchiveStats {
    pub total: usize,
    pub completed: usize,
    pub errors: usize,
    pub detail: String,
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .get(key)
        .or_else(|| value.pointer("/attributes").and_then(|v| v.get(key)))
}
fn text(value: &Value, key: &str) -> String {
    field(value, key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
fn resource(value: &Value) -> &Value {
    value.get("data").filter(|v| v.is_object()).unwrap_or(value)
}
fn rows(value: &Value) -> Result<&Vec<Value>, String> {
    value
        .get("items")
        .or_else(|| value.pointer("/data/items"))
        .or_else(|| value.get("data"))
        .or_else(|| value.is_array().then_some(value))
        .and_then(Value::as_array)
        .ok_or("Build API returned no item list.".into())
}
fn paginated<C: BuildApi>(
    client: &mut C,
    operation: &str,
    mut args: Value,
) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    loop {
        let response = client.call(operation, args.clone())?;
        result.extend(rows(&response)?.iter().cloned());
        let next = response
            .get("next_page")
            .or_else(|| response.get("nextPage"))
            .filter(|v| !v.is_null() && v.as_str() != Some(""));
        let Some(next) = next else {
            if response.get("complete") == Some(&Value::Bool(false)) {
                return Err(
                    "Build API reports an incomplete page without a continuation cursor.".into(),
                );
            }
            break;
        };
        if !seen.insert(next.to_string()) {
            return Err("Build API repeated a page cursor.".into());
        }
        args["page"] = next.clone();
    }
    Ok(result)
}
fn team(config: &ImageHubConfig) -> Result<String, String> {
    if let Some(team) = config.extra.get("build_team").filter(|v| !v.is_empty()) {
        return Ok(team.clone());
    }
    let url = config
        .extra
        .get("build_log_url")
        .and_then(|url| url::Url::parse(url).ok())
        .ok_or("Set build_team or build_log_url in Image Hub → Config.")?;
    let parts = url
        .path_segments()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    parts
        .windows(2)
        .find(|pair| pair[0] == "teams")
        .map(|pair| pair[1].to_owned())
        .filter(|v| !v.is_empty())
        .ok_or("The build team could not be read from build_log_url.".into())
}
fn image_metadata(detail: &Value) -> Result<Vec<Value>, String> {
    let item = resource(detail);
    let metadata = field(item, "stage_variables").and_then(|v| v.get("ARTIFACT_METADATA"));
    match metadata {
        Some(Value::String(value)) => serde_json::from_str::<Vec<Value>>(value)
            .map_err(|_| "Build ARTIFACT_METADATA is not valid JSON.".into()),
        Some(Value::Array(values)) => Ok(values.clone()),
        None | Some(Value::Null) => Ok(Vec::new()),
        _ => Err("Unsupported build artifact metadata format.".into()),
    }
}
fn terminal(detail: &Value) -> bool {
    let item = resource(detail);
    matches!(
        text(item, "status").to_ascii_uppercase().as_str(),
        "DONE" | "COMPLETED" | "FAILED" | "CANCELLED" | "ABORTED"
    )
}
fn cached(
    db: &Connection,
    repo: &str,
    source: &str,
    guid: &str,
    summary: &Value,
) -> rusqlite::Result<bool> {
    Ok(db.query_row("SELECT complete,status,COALESCE(result,'') FROM image_hub_builds WHERE repository_id=?1 AND source=?2 AND build_guid=?3",params![repo,source,guid],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?.is_some_and(|(complete,status,result)|complete&&status==text(summary,"status")&&result==text(summary,"result")))
}
fn store_build(
    db: &Connection,
    project: &str,
    repo: &str,
    source: &str,
    guid: &str,
    detail: &Value,
) -> rusqlite::Result<String> {
    let item = resource(detail);
    let attrs = item.get("attributes").unwrap_or(item);
    let proposed = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO image_hub_builds(id,project_id,repository_id,source,build_guid,build_number,status,result,commit_hash,time_started,time_ended,raw_json,complete,error,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,0,NULL,?13) ON CONFLICT(repository_id,source,build_guid) DO UPDATE SET build_number=excluded.build_number,status=excluded.status,result=excluded.result,commit_hash=excluded.commit_hash,time_started=excluded.time_started,time_ended=excluded.time_ended,raw_json=excluded.raw_json,complete=0,error=NULL,fetched_at=excluded.fetched_at",params![proposed,project,repo,source,guid,attrs.get("build_number").and_then(Value::as_i64),text(attrs,"status"),text(attrs,"result"),text(attrs,"commit_hash"),text(attrs,"time_started"),text(attrs,"time_ended"),detail.to_string(),super::artifacts::now()])?;
    db.query_row(
        "SELECT id FROM image_hub_builds WHERE repository_id=?1 AND source=?2 AND build_guid=?3",
        params![repo, source, guid],
        |r| r.get(0),
    )
}
fn store_images(db: &Connection, build: &str, images: &[Value]) -> rusqlite::Result<()> {
    for image in images {
        if !text(image, "artifactType").eq_ignore_ascii_case("docker") {
            continue;
        }
        let name = text(image, "artifactName");
        let version = text(image, "artifactVersion");
        if name.is_empty() || version.is_empty() {
            continue;
        }
        db.execute("INSERT OR IGNORE INTO image_hub_build_images(build_id,image_name,version,digest,artifact_url) VALUES(?1,?2,?3,?4,?5)",params![build,name,version,text(image,"artifactDigest"),text(image,"artifactUrl")])?;
    }
    Ok(())
}
fn coordinates(node: &Value, result: &mut Vec<Value>) {
    match node {
        Value::Array(nodes) => {
            for node in nodes {
                coordinates(node, result);
            }
        }
        Value::Object(nodes) => {
            for (key, value) in nodes {
                if matches!(key.as_str(), "log_coordinates" | "logCoordinates") && value.is_object()
                {
                    result.push(value.clone());
                } else if value.is_array() || value.is_object() {
                    coordinates(value, result);
                }
            }
        }
        _ => {}
    }
}
fn coordinate_text(value: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .unwrap_or("")
        .to_owned()
}
fn log_state(
    db: &Connection,
    build: &str,
    step: &str,
    coordinate: &Value,
) -> rusqlite::Result<(String, i64, bool)> {
    let namespace = coordinate_text(coordinate, &["namespace"]);
    let bucket = coordinate_text(coordinate, &["bucket", "bucket_name"]);
    let object = coordinate_text(coordinate, &["objectName", "object_name"]);
    if namespace.is_empty() || bucket.is_empty() || object.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Incomplete build log coordinates.".into(),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT OR IGNORE INTO image_hub_build_logs(id,build_id,step_guid,namespace,bucket,object_name,raw_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,build,step,namespace,bucket,object,coordinate.to_string()])?;
    db.query_row("SELECT id,next_byte,complete FROM image_hub_build_logs WHERE build_id=?1 AND step_guid=?2 AND namespace=?3 AND bucket=?4 AND object_name=?5",params![build,step,namespace,bucket,object],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
}
fn store_chunk(
    db: &Connection,
    log: &str,
    start: i64,
    next: i64,
    content: &str,
    complete: bool,
) -> rusqlite::Result<()> {
    let transaction = db.unchecked_transaction()?;
    let expected: i64 = transaction.query_row(
        "SELECT next_byte FROM image_hub_build_logs WHERE id=?1",
        [log],
        |r| r.get(0),
    )?;
    if expected != start || next < start {
        return Err(rusqlite::Error::InvalidParameterName(
            "Build log cursor changed unexpectedly.".into(),
        ));
    }
    if next > start {
        transaction.execute("INSERT INTO image_hub_build_log_chunks(log_id,start_byte,next_byte,content) VALUES(?1,?2,?3,?4)",params![log,start,next,content])?;
    }
    transaction.execute(
        "UPDATE image_hub_build_logs SET next_byte=?2,complete=?3 WHERE id=?1",
        params![log, next, complete],
    )?;
    transaction.commit()
}
fn archive_build<C: BuildApi, D: Database>(
    client: &mut C,
    db: &D,
    project: &str,
    repo: &Repository,
    source: &str,
    guid: &str,
    config: &ImageHubConfig,
) -> Result<(), String> {
    let detail = client.call("get_build", json!({"build_guid":guid}))?;
    let returned = text(resource(&detail), "id");
    if !returned.is_empty() && returned != guid {
        return Err("Build API returned another build's metadata.".into());
    }
    let build = db.with(|db| store_build(db, project, &repo.id, source, guid, &detail))?;
    let operation = (|| {
        let images = image_metadata(&detail)?;
        db.with(|db| store_images(db, &build, &images))?;
        let done = terminal(&detail);
        let steps = paginated(
            client,
            "get_build_steps",
            json!({"build_guid":guid,"generate_par_link":false,"limit":100}),
        )?;
        let mut seen = HashSet::new();
        for step in steps {
            let step_guid = text(&step, "step_guid");
            if step_guid.is_empty() {
                return Err("Build step has no identifier.".into());
            }
            if !seen.insert(step_guid.clone()) {
                continue;
            }
            let metadata = client.call(
                "get_step_artifacts",
                json!({"step_guid":step_guid,"generate_par_link":false}),
            )?;
            let mut logs = Vec::new();
            coordinates(&metadata, &mut logs);
            if logs.is_empty()
                && !matches!(
                    text(&step, "status").to_ascii_uppercase().as_str(),
                    "SKIPPED" | "NOT_STARTED" | "PENDING"
                )
            {
                return Err(format!(
                    "No log coordinates available for build step {step_guid}."
                ));
            }
            for coordinate in logs {
                let (log, mut start, complete) =
                    db.with(|db| log_state(db, &build, &step_guid, &coordinate))?;
                if complete {
                    continue;
                }
                loop {
                    let mut args = json!({"namespace":coordinate_text(&coordinate,&["namespace"]),"bucket_name":coordinate_text(&coordinate,&["bucket","bucket_name"]),"object_name":coordinate_text(&coordinate,&["objectName","object_name"]),"start_byte":start,"max_bytes":262144});
                    let region = coordinate
                        .get("region")
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                        .or_else(|| {
                            config
                                .extra
                                .get("build_region")
                                .map(String::as_str)
                                .filter(|v| !v.is_empty())
                        })
                        .or_else(|| (!config.region.is_empty()).then_some(config.region.as_str()));
                    if let Some(region) = region {
                        args["region"] = json!(region);
                    }
                    let page = client.call("download_logs", args)?;
                    let content = ["content", "text", "log_text", "logs"]
                        .iter()
                        .find_map(|key| page.get(*key).and_then(Value::as_str))
                        .ok_or("Build API returned no text log page.")?;
                    let truncated = page
                        .get("truncated")
                        .and_then(Value::as_bool)
                        .ok_or("Build API log page has no truncation status.")?;
                    let next = page
                        .get("next_start_byte")
                        .and_then(Value::as_i64)
                        .or_else(|| (!truncated).then_some(start + content.len() as i64))
                        .ok_or("Build log page has no continuation byte offset.")?;
                    if next < start || (truncated && next <= start) {
                        return Err("Build API repeated a log byte offset.".into());
                    }
                    db.with(|db| store_chunk(db, &log, start, next, content, !truncated && done))?;
                    start = next;
                    if !truncated {
                        break;
                    }
                }
            }
        }
        db.with(|db| {
            db.execute(
                "UPDATE image_hub_builds SET complete=?2,error=NULL WHERE id=?1",
                params![build, done],
            )?;
            Ok(())
        })?;
        Ok(())
    })();
    if let Err(error) = &operation {
        let _ = db.with(|db| {
            db.execute(
                "UPDATE image_hub_builds SET error=?2 WHERE id=?1",
                params![build, error],
            )?;
            Ok(())
        });
    }
    operation
}
pub fn archive<C: BuildApi, D: Database>(
    client: &mut C,
    db: &D,
    project: &str,
    repos: &[Repository],
    source: &str,
    config: &ImageHubConfig,
    mut progress: impl FnMut(usize, usize, usize, &str) -> Result<(), String>,
) -> Result<ArchiveStats, String> {
    let team = team(config)?;
    let mut stats = ArchiveStats::default();
    let mut work = Vec::new();
    let mut seen = HashSet::new();
    for repo in repos {
        progress(
            0,
            0,
            0,
            &format!("Discovering build history: {}", repo.name),
        )?;
        match paginated(
            client,
            "list_builds",
            json!({"team_name":team,"project_name":repo.name,"time_started":"1970-01-01","limit":100}),
        ) {
            Ok(rows) => {
                for row in rows {
                    let guid = text(&row, "build_guid");
                    let guid = if guid.is_empty() {
                        text(&row, "id")
                    } else {
                        guid
                    };
                    if guid.is_empty() {
                        return Err(
                            "Build list contains an item without a build identifier.".into()
                        );
                    }
                    if seen.insert((repo.id.clone(), guid.clone())) {
                        work.push((repo, guid, row));
                    }
                }
            }
            Err(error) => {
                stats.errors += 1;
                stats.detail = format!("{}: {error}", repo.name);
            }
        }
    }
    stats.total = work.len() + stats.errors;
    stats.completed = stats.errors;
    for (repo, guid, summary) in work {
        if !db.with(|db| cached(db, &repo.id, source, &guid, &summary))? {
            if let Err(error) = archive_build(client, db, project, repo, source, &guid, config) {
                stats.errors += 1;
                stats.detail = format!("{} / {guid}: {error}", repo.name);
            }
        }
        stats.completed += 1;
        progress(stats.completed, stats.total, stats.errors, &stats.detail)?;
    }
    Ok(stats)
}
pub fn targets(
    db: &Connection,
    project: &str,
    source: &str,
) -> rusqlite::Result<BTreeSet<(String, String)>> {
    let mut statement=db.prepare("SELECT DISTINCT i.image_name,i.version FROM image_hub_build_images i JOIN image_hub_builds b ON b.id=i.build_id WHERE b.project_id=?1 AND b.source=?2")?;
    statement
        .query_map(params![project, source], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}
pub fn link_security(
    db: &Connection,
    project: &str,
    image: &str,
    version: &str,
    report: &str,
) -> rusqlite::Result<()> {
    db.execute("UPDATE image_hub_build_images SET security_report_id=?4 WHERE image_name=?2 AND version=?3 AND build_id IN(SELECT id FROM image_hub_builds WHERE project_id=?1)",params![project,image,version,report])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Db(RefCell<Connection>);
    impl Database for Db {
        fn with<T>(
            &self,
            operation: impl FnOnce(&Connection) -> rusqlite::Result<T>,
        ) -> Result<T, String> {
            operation(&self.0.borrow()).map_err(|e| e.to_string())
        }
    }
    struct Api {
        calls: Vec<(String, Value)>,
        fail_once: bool,
    }
    impl BuildApi for Api {
        fn call(&mut self, op: &str, args: Value) -> Result<Value, String> {
            self.calls.push((op.into(), args.clone()));
            Ok(match op {
                "list_builds" => {
                    assert_eq!(args["time_started"], "1970-01-01");
                    if args.get("page").is_none() {
                        json!({"items":[{"build_guid":"new","status":"DONE","result":"SUCCEEDED"},{"build_guid":"old","status":"DONE","result":"SUCCEEDED"}],"next_page":"second","complete":false})
                    } else {
                        json!({"items":[{"build_guid":"failed","status":"DONE","result":"FAILED"}],"next_page":null,"complete":true})
                    }
                }
                "get_build" => {
                    let guid = args["build_guid"].as_str().unwrap();
                    let artifacts = if guid == "failed" {
                        json!([])
                    } else {
                        json!([{"artifactType":"DOCKER","artifactName":"sample","artifactVersion":if guid=="new"{"1.0.0"}else{"0.9.0"},"artifactDigest":"sha256:example","artifactUrl":"https://artifacts.example.test/image"}])
                    };
                    json!({"data":{"id":guid,"attributes":{"status":"DONE","result":if guid=="failed"{"FAILED"}else{"SUCCEEDED"},"build_number":1,"stage_variables":{"ARTIFACT_METADATA":artifacts.to_string()}}}})
                }
                "get_build_steps" => {
                    json!({"items":[{"step_guid":args["build_guid"],"status":"DONE"}],"complete":true,"next_page":null})
                }
                "get_step_artifacts" => {
                    assert_eq!(args["generate_par_link"], false);
                    json!({"log_coordinates":{"namespace":"logs","bucket":"builds","objectName":args["step_guid"]}})
                }
                "download_logs" => {
                    let offset = args["start_byte"].as_i64().unwrap();
                    if args["object_name"] == "new" && offset == 3 && self.fail_once {
                        self.fail_once = false;
                        return Err("Temporary log failure".into());
                    }
                    if offset == 0 {
                        json!({"content":"abc","truncated":true,"next_start_byte":3})
                    } else {
                        assert_eq!(offset, 3);
                        json!({"content":"def","truncated":false,"next_start_byte":6})
                    }
                }
                _ => return Err("Unexpected operation".into()),
            })
        }
    }
    #[test]
    fn all_builds_versions_and_log_pages_are_archived_resumably_without_changing_latest() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::storage::migrations::run_migrations(&mut connection).unwrap();
        connection.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Test','/tmp/test','scm','')",[]).unwrap();
        connection.execute("INSERT INTO image_hub_repositories(id,name,repo_type,project_id) VALUES('repo','sample','scm','project')",[]).unwrap();
        connection.execute("INSERT INTO image_hub_latest_images(repository_id,image_name,version,status,checked_at) VALUES('repo','sample','9.0.0','found',1)",[]).unwrap();
        let db = Db(RefCell::new(connection));
        let repo = Repository {
            id: "repo".into(),
            folder_id: None,
            name: "sample".into(),
            local_path: "/tmp/test/sample".into(),
            ocid: String::new(),
            url: String::new(),
            repo_type: "scm".into(),
            browser_url: None,
            project_id: Some("project".into()),
        };
        let mut config = ImageHubConfig::default();
        config
            .extra
            .insert("build_team".into(), "sample-team".into());
        let mut api = Api {
            calls: Vec::new(),
            fail_once: true,
        };
        let first = archive(
            &mut api,
            &db,
            "project",
            &[repo.clone()],
            "http://mcp.example.test/mcp",
            &config,
            |_, _, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(first.total, 3);
        assert_eq!(first.errors, 1);
        let second = archive(
            &mut api,
            &db,
            "project",
            &[repo.clone()],
            "http://mcp.example.test/mcp",
            &config,
            |_, _, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(second.errors, 0);
        assert_eq!(second.completed, 3);
        assert_eq!(
            api.calls
                .iter()
                .filter(|(op, args)| op == "download_logs"
                    && args["object_name"] == "new"
                    && args["start_byte"] == 0)
                .count(),
            1
        );
        let requests = api
            .calls
            .iter()
            .filter(|(op, _)| op == "download_logs")
            .count();
        archive(
            &mut api,
            &db,
            "project",
            &[repo],
            "http://mcp.example.test/mcp",
            &config,
            |_, _, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(
            api.calls
                .iter()
                .filter(|(op, _)| op == "download_logs")
                .count(),
            requests
        );
        db.with(|db|{
            assert_eq!(db.query_row("SELECT COUNT(*) FROM image_hub_builds WHERE complete=1",[],|r|r.get::<_,i64>(0))?,3);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM image_hub_build_log_chunks",[],|r|r.get::<_,i64>(0))?,6);
            assert_eq!(db.query_row("SELECT version FROM image_hub_latest_images",[],|r|r.get::<_,String>(0))?,"9.0.0");
            assert_eq!(targets(db,"project","http://mcp.example.test/mcp")?,BTreeSet::from([("sample".into(),"0.9.0".into()),("sample".into(),"1.0.0".into())]));
            assert!(targets(db,"project","http://other.example.test/mcp")?.is_empty());
            db.execute("INSERT INTO image_hub_security_reports(id,project_id,image_name,version,realm,scanner,source_url,decision,fetched_at) VALUES('scan','project','sample','1.0.0','TEST','anchore','https://security.example.test','unknown',1)",[])?;
            link_security(db,"project","sample","1.0.0","scan")?;
            assert_eq!(db.query_row("SELECT COUNT(*) FROM image_hub_build_images WHERE security_report_id='scan'",[],|r|r.get::<_,i64>(0))?,1);
            Ok(())
        }).unwrap();
    }
    #[test]
    fn repeated_log_offsets_and_page_cursors_do_not_silently_truncate_history() {
        struct Repeated;
        impl BuildApi for Repeated {
            fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
                Ok(json!({"items":[],"next_page":"same","complete":false}))
            }
        }
        assert!(
            paginated(&mut Repeated, "list_builds", json!({}))
                .unwrap_err()
                .contains("repeated")
        );
        assert!(
            image_metadata(
                &json!({"data":{"attributes":{"stage_variables":{"ARTIFACT_METADATA":"not json"}}}})
            )
            .is_err()
        );
    }
}
