use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReport {
    pub id: String,
    pub project_id: String,
    pub image_name: String,
    pub version: String,
    pub realm: String,
    pub scanner: String,
    pub source_url: String,
    pub decision: String,
    pub finding_count: Option<i64>,
    pub scan_time: Option<String>,
    pub fetched_at: i64,
    pub error: Option<String>,
    #[serde(skip_serializing)]
    pub raw: Option<Value>,
}

pub fn authentication_required(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "oci_auth_required",
        "session has expired",
        "security token expired",
        "tokenexpired",
        "expiredtoken",
        "notauthenticated",
        "authentication failed",
        "401 unauthorized",
        "invalid security token",
        "token has expired",
        "security_token_file",
    ]
    .iter()
    .any(|pattern| error.contains(pattern))
}

pub fn auth_help(config: &super::config::ImageHubConfig) -> String {
    let profile = config
        .extra
        .get("scm_profile")
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(if config.scm_profile.is_empty() {
            "OCI CLI default"
        } else {
            &config.scm_profile
        });
    let mut message = format!(
        "OCI authentication was rejected or is unavailable for profile {profile}. Check scm_profile and scm_auth, refresh that profile in your terminal if needed, then run Sync Project again."
    );
    if let Some(command) = config
        .extra
        .get("security_auth_command")
        .filter(|v| !v.trim().is_empty())
    {
        message.push_str("\nRun: ");
        message.push_str(command);
    }
    message
}

fn value(node: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        for source in [Some(node), node.get("metadata")] {
            if let Some(value) = source.and_then(|source| source.get(*key)) {
                match value {
                    Value::String(value) => return Some(value.clone()),
                    Value::Number(value) => return Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }
    None
}

fn findings<'a>(node: &'a Value, result: &mut Vec<&'a Value>) {
    match node {
        Value::Array(nodes) => {
            for node in nodes {
                findings(node, result);
            }
        }
        Value::Object(nodes) => {
            if value(
                node,
                &[
                    "CVE",
                    "cve",
                    "vulnerabilityId",
                    "findingId",
                    "uniqueFindingIdentifier",
                    "Gating_Date",
                    "gatingDate",
                ],
            )
            .is_some()
            {
                result.push(node);
            }
            for key in ["findings", "vulnerabilities", "matches", "results"] {
                if let Some(child) = nodes.get(key) {
                    findings(child, result);
                }
            }
        }
        _ => {}
    }
}

pub fn store(db: &Connection, report: &SecurityReport) -> rusqlite::Result<()> {
    let transaction = db.unchecked_transaction()?;
    transaction.execute("INSERT INTO image_hub_security_reports(id,project_id,image_name,version,realm,scanner,source_url,decision,scan_time,fetched_at,error,raw_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![report.id, report.project_id, report.image_name, report.version, report.realm, report.scanner, report.source_url, report.decision, report.scan_time, report.fetched_at, report.error, report.raw.as_ref().map(Value::to_string)])?;
    if let Some(raw) = &report.raw {
        if let Some(parts) = raw.as_object() {
            for (name, part) in parts {
                transaction.execute("INSERT INTO image_hub_security_parts(report_id,name,raw_json) VALUES(?1,?2,?3)", params![report.id, name, part.to_string()])?;
            }
        } else {
            transaction.execute("INSERT INTO image_hub_security_parts(report_id,name,raw_json) VALUES(?1,'report',?2)", params![report.id, raw.to_string()])?;
        }
        let mut rows = Vec::new();
        findings(raw, &mut rows);
        for (index, finding) in rows.iter().enumerate() {
            transaction.execute("INSERT INTO image_hub_security_findings(report_id,finding_index,finding_id,severity,package,package_type,installed_version,fixed_version,gating_date,raw_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![report.id, index as i64,
                value(finding, &["CVE", "cve", "vulnerabilityId", "findingId", "Advisory_Name", "primary_vulnerability_id", "uniqueFindingIdentifier", "id"]), value(finding, &["Severity", "severity"]),
                value(finding, &["Package_Name", "packageName", "package", "PURL", "purl"]), value(finding, &["packageType", "package_type", "type"]).or_else(|| value(finding, &["PURL", "purl"]).and_then(|purl| purl.strip_prefix("pkg:").and_then(|v| v.split('/').next()).map(str::to_owned))),
                value(finding, &["Package_Version", "installedVersion", "packageVersion", "version"]), value(finding, &["CVE_Fix_Version", "fixedVersion", "fix"]),
                value(finding, &["Gating_Date", "gatingDate", "gating_date"]), finding.to_string()])?;
        }
    }
    transaction.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn raw_reports_parts_and_searchable_findings_persist_with_history() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Sample','/sample','scm','')", []).unwrap();
        let report = SecurityReport {
            id: "scan1".into(),
            project_id: "project".into(),
            image_name: "sample".into(),
            version: "1.2.3".into(),
            realm: "TEST".into(),
            scanner: "anchore".into(),
            source_url: "https://security.example.test/report".into(),
            decision: "denied".into(),
            finding_count: None,
            scan_time: None,
            fetched_at: 1,
            error: None,
            raw: Some(
                json!({"findings": [{"metadata": {"CVE": "CVE-TEST", "PURL": "pkg:golang/example", "Severity": "High", "Gating_Date": "IMMEDIATELY GATING"}}], "metadata": {"unknown": "preserved"}, "scanFileInfo": {"digest": "sha256:test"}}),
            ),
        };
        store(&db, &report).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM image_hub_security_parts", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            3
        );
        assert_eq!(db.query_row("SELECT package FROM image_hub_security_findings WHERE finding_id='CVE-TEST' AND severity='High'", [], |r| r.get::<_,String>(0)).unwrap(), "pkg:golang/example");
        let mut second = report.clone();
        second.id = "scan2".into();
        second.fetched_at = 2;
        store(&db, &second).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM image_hub_security_reports", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert!(store(&db, &second).is_err());
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM image_hub_security_findings",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }
    #[test]
    fn expired_authentication_is_distinguished_from_denied_policy_and_network_errors() {
        assert!(authentication_required(
            "ERROR: This CLI session has expired"
        ));
        assert!(authentication_required("ServiceError: NotAuthenticated"));
        assert!(!authentication_required("Policy decision Denied"));
        assert!(!authentication_required("Connection timed out"));
        let mut config = super::super::config::ImageHubConfig::default();
        config.extra.insert(
            "security_auth_command".into(),
            "/tools/gateway refresh".into(),
        );
        assert!(auth_help(&config).contains("/tools/gateway refresh"));
    }
}

pub(super) fn api_settings(
    config: &super::config::ImageHubConfig,
) -> Result<(url::Url, String), String> {
    let base = config
        .extra
        .get("slaps_api_url")
        .filter(|v| !v.trim().is_empty())
        .ok_or(
            "Set slaps_api_url and slaps_realm in Image Hub → Config to download security reports.",
        )?;
    let url = url::Url::parse(base).map_err(|_| "Invalid slaps_api_url.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "slaps_api_url must be an HTTPS base URL without credentials, query or fragment."
                .into(),
        );
    }
    let realm = config
        .extra
        .get("slaps_realm")
        .filter(|v| !v.trim().is_empty())
        .ok_or("Set slaps_realm in Image Hub → Config.")?
        .trim()
        .to_uppercase();
    Ok((url, realm))
}

fn scan_url(
    config: &super::config::ImageHubConfig,
    image: &str,
    version: &str,
) -> Result<url::Url, String> {
    let (mut url, _) = api_settings(config)?;
    url.set_path(&format!(
        "{}/slaps/v1/audit/artifact/scanreport",
        url.path().trim_end_matches('/')
    ));
    url.query_pairs_mut()
        .append_pair("name", image)
        .append_pair("version", version)
        .append_pair("artifactType", "docker")
        .append_pair("scannerType", "anchore");
    Ok(url)
}

fn request(url: &str, config: &super::config::ImageHubConfig) -> Result<Value, String> {
    if config.scm_profile.is_empty() || config.scm_auth.is_empty() {
        return Err("Set scm_profile and scm_auth in Image Hub → Config for security report authentication.".into());
    }
    let args = [
        "raw-request",
        "--http-method",
        "GET",
        "--target-uri",
        url,
        "--profile",
        &config.scm_profile,
        "--auth",
        &config.scm_auth,
        "--output",
        "json",
    ];
    let text = super::project_sync::run("oci", &args, None)?;
    let envelope: Value =
        serde_json::from_str(&text).map_err(|_| "Security API returned invalid JSON.")?;
    response_data(envelope, config)
}

fn response_data(envelope: Value, config: &super::config::ImageHubConfig) -> Result<Value, String> {
    let status = envelope.get("status").and_then(Value::as_str).unwrap_or("");
    if status.starts_with("401") {
        let code = envelope
            .pointer("/data/code")
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() <= 80
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .unwrap_or("no service error code");
        return Err(format!(
            "OCI_AUTH_REQUIRED: Security API returned HTTP 401 ({code}; profile: {}; auth: {}).",
            config.scm_profile, config.scm_auth
        ));
    }
    if !status.starts_with("200") {
        let code = envelope
            .pointer("/data/code")
            .and_then(Value::as_str)
            .filter(|code| {
                code.len() <= 80 && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
        let detail = if code == Some("RelatedResourceNotAuthorizedOrNotFound") {
            " The artifact/version is unavailable or inaccessible to the selected OCI identity. Check the version configured in Ansible and artifact access."
        } else {
            ""
        };
        return Err(format!(
            "Security API returned HTTP {}{}.{}",
            status.split_whitespace().next().unwrap_or("unknown"),
            code.map(|code| format!(" ({code})")).unwrap_or_default(),
            detail,
        ));
    }
    Ok(envelope.get("data").cloned().unwrap_or(envelope))
}

fn decode_scan(response: Value) -> Result<Option<Value>, String> {
    let mut report = response
        .get("scanReport")
        .or_else(|| response.get("scan_report"))
        .or_else(|| response.get("report"))
        .cloned()
        .unwrap_or(response);
    if report.is_null() || report.as_str().is_some_and(|v| v.is_empty() || v == "null") {
        return Ok(None);
    }
    if let Some(text) = report.as_str() {
        report =
            serde_json::from_str(text).map_err(|_| "Security scanReport contains invalid JSON.")?;
    }
    if !report.is_object() && !report.is_array() {
        return Err("Unexpected security scan report format.".into());
    }
    Ok(Some(report))
}

pub fn fetch(
    project_id: &str,
    image: &str,
    version: &str,
    config: &super::config::ImageHubConfig,
) -> Result<SecurityReport, String> {
    let (_, realm) = api_settings(config)?;
    let url = scan_url(config, image, version)?;
    let mut result = SecurityReport {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.into(),
        image_name: image.into(),
        version: version.into(),
        realm,
        scanner: "anchore".into(),
        source_url: url.to_string(),
        decision: "unknown".into(),
        finding_count: None,
        scan_time: None,
        fetched_at: super::artifacts::now(),
        error: None,
        raw: None,
    };
    match request(url.as_str(), config).and_then(decode_scan) {
        Ok(Some(report)) => {
            result.scan_time = value(
                &report,
                &["scan_completed_at", "scanTimestamp", "generatedAt"],
            );
            result.raw = Some(report);
        }
        Ok(None) => {
            result.decision = "pending".into();
        }
        Err(error) => {
            result.decision = "error".into();
            result.error = Some(error);
        }
    }
    Ok(result)
}

pub fn load_summaries(
    db: &Connection,
    config: &super::config::ImageHubConfig,
) -> rusqlite::Result<Vec<SecurityReport>> {
    let Ok((_, realm)) = api_settings(config) else {
        return Ok(Vec::new());
    };
    let mut statement = db.prepare("SELECT id,project_id,image_name,version,realm,scanner,source_url,decision,scan_time,fetched_at,error,CASE WHEN raw_json IS NOT NULL THEN (SELECT COUNT(*) FROM image_hub_security_findings WHERE report_id=reports.id) END FROM (SELECT *,ROW_NUMBER() OVER(PARTITION BY project_id,image_name,version,realm,scanner,source_url ORDER BY fetched_at DESC,rowid DESC) AS rank FROM image_hub_security_reports WHERE realm=?1) AS reports WHERE rank=1")?;
    let reports = statement
        .query_map([realm], |r| {
            Ok(SecurityReport {
                id: r.get(0)?,
                project_id: r.get(1)?,
                image_name: r.get(2)?,
                version: r.get(3)?,
                realm: r.get(4)?,
                scanner: r.get(5)?,
                source_url: r.get(6)?,
                decision: r.get(7)?,
                finding_count: r.get(11)?,
                scan_time: r.get(8)?,
                fetched_at: r.get(9)?,
                error: r.get(10)?,
                raw: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(reports
        .into_iter()
        .filter(|r| {
            scan_url(config, &r.image_name, &r.version)
                .ok()
                .is_some_and(|url| url.as_str() == r.source_url)
        })
        .collect())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingRow {
    pub finding_id: Option<String>,
    pub severity: Option<String>,
    pub package: Option<String>,
    pub installed_version: Option<String>,
    pub fixed_version: Option<String>,
    pub gating_date: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPage {
    pub report: Option<SecurityReport>,
    pub findings: Vec<FindingRow>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

pub fn page(
    db: &Connection,
    config: &super::config::ImageHubConfig,
    project_id: &str,
    image: &str,
    version: &str,
    page: i64,
    page_size: i64,
) -> rusqlite::Result<ReportPage> {
    let report = load_summaries(db, config)?
        .into_iter()
        .find(|r| r.project_id == project_id && r.image_name == image && r.version == version);
    let page_size = page_size.clamp(1, 20);
    let Some(report) = report else {
        return Ok(ReportPage {
            report: None,
            findings: vec![],
            total: 0,
            page: 0,
            page_size,
        });
    };
    let total = db.query_row(
        "SELECT COUNT(*) FROM image_hub_security_findings WHERE report_id=?1",
        [&report.id],
        |r| r.get::<_, i64>(0),
    )?;
    let page = page.clamp(0, ((total - 1).max(0)) / page_size);
    let mut statement = db.prepare("SELECT finding_id,severity,package,installed_version,fixed_version,gating_date FROM image_hub_security_findings WHERE report_id=?1 ORDER BY CASE LOWER(severity) WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END,finding_id,finding_index LIMIT ?2 OFFSET ?3")?;
    let findings = statement
        .query_map(params![report.id, page_size, page * page_size], |r| {
            Ok(FindingRow {
                finding_id: r.get(0)?,
                severity: r.get(1)?,
                package: r.get(2)?,
                installed_version: r.get(3)?,
                fixed_version: r.get(4)?,
                gating_date: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ReportPage {
        report: Some(report),
        findings,
        total,
        page,
        page_size,
    })
}

#[cfg(test)]
mod report_tests {
    use super::*;
    #[test]
    fn http_status_not_report_content_controls_api_authentication_errors() {
        let config = settings().resolved().unwrap();
        let ok = serde_json::json!({"status":"200 OK","data":{"scanReport":{"findings":[{"details":"authentication failed security_token_file"}]}}});
        assert!(response_data(ok, &config).is_ok());
        let error = response_data(
            serde_json::json!({"status":"401 Unauthorized","data":{"code":"NotAuthenticated"}}),
            &config,
        )
        .unwrap_err();
        assert!(authentication_required(&error));
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("TEST_SESSION"));
        assert!(!authentication_required(
            &response_data(serde_json::json!({"status":"403 Forbidden"}), &config).unwrap_err()
        ));
    }
    #[test]
    fn unavailable_artifact_errors_show_service_code_without_exposing_response_details() {
        let config = settings().resolved().unwrap();
        let error = response_data(
            json!({"status":"400 Bad Request", "data":{
                "code":"RelatedResourceNotAuthorizedOrNotFound",
                "message":"Error while authorizing the user: private-user-id"
            }}),
            &config,
        )
        .unwrap_err();
        assert!(error.contains("HTTP 400 (RelatedResourceNotAuthorizedOrNotFound)"));
        assert!(error.contains("Ansible"));
        assert!(!error.contains("private-user-id"));
        assert!(!authentication_required(&error));
        let plain = response_data(json!({"status":"404 Not Found"}), &config).unwrap_err();
        assert_eq!(plain, "Security API returned HTTP 404.");
    }

    #[test]
    #[ignore = "Requires an external configuration file and existing security-token session"]
    fn configured_security_request_live() {
        let raw: super::super::config::ImageHubConfig = serde_json::from_slice(
            &std::fs::read(std::env::var("GATETREE_SECURITY_TEST_CONFIG").unwrap()).unwrap(),
        )
        .unwrap();
        let config = raw.resolved().unwrap();
        let image = std::env::var("GATETREE_SECURITY_TEST_IMAGE").unwrap();
        let version = std::env::var("GATETREE_SECURITY_TEST_VERSION").unwrap();
        let report = fetch("project", &image, &version, &config).unwrap();
        assert!(report.error.is_none(), "{:?}", report.error);
        assert!(
            report.raw.is_some(),
            "The selected image must have a report"
        );
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Test','/tmp/test','scm','')",[]).unwrap();
        store(&db, &report).unwrap();
        let result = page(&db, &config, "project", &image, &version, 0, 20).unwrap();
        assert!(result.report.is_some());
        println!(
            "Native GT request downloaded and stored {} findings in {}.",
            result.total, report.realm
        );
    }

    use serde_json::json;
    fn settings() -> super::super::config::ImageHubConfig {
        serde_json::from_value(json!({"slaps_api_url":"https://security.example.test", "slaps_realm":"TEST", "scm_profile":"TEST_SESSION", "scm_auth":"security_token"})).unwrap()
    }
    #[test]
    fn decoded_reports_are_paged_searchable_and_never_imply_approval() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Test','/tmp/test','scm','')",[]).unwrap();
        let config = settings();
        let scan = json!({"metadata":{"scan_completed_at":"2026-11-03T10:00:00Z"},"findings":[
            {"uniqueFindingIdentifier":"long-scanner-prefix-for-first-finding","severity":"HIGH","metadata":{"Advisory_Name":"GHSA-test","Package_Name":"example/module","Package_Version":"1.0","CVE_Fix_Version":"1.1","PURL":"pkg:golang/example/module@1.0","Gating_Date":"IMMEDIATELY GATING"}},
            {"uniqueFindingIdentifier":"other","severity":"LOW","metadata":{"Advisory_Name":"CVE-example"}}
        ]});
        let decoded = decode_scan(json!({"scanReport":scan.to_string()}))
            .unwrap()
            .unwrap();
        let report = SecurityReport {
            id: "report".into(),
            project_id: "project".into(),
            image_name: "sample".into(),
            version: "1.2.3".into(),
            realm: "TEST".into(),
            scanner: "anchore".into(),
            source_url: scan_url(&config, "sample", "1.2.3").unwrap().to_string(),
            decision: "unknown".into(),
            finding_count: None,
            scan_time: None,
            fetched_at: 1,
            error: None,
            raw: Some(decoded),
        };
        store(&db, &report).unwrap();
        let summary = &load_summaries(&db, &config).unwrap()[0];
        assert_eq!(summary.finding_count, Some(2));
        assert_eq!(serde_json::to_value(summary).unwrap()["findingCount"], 2);
        let mut empty = report.clone();
        empty.id = "empty".into();
        empty.version = "2.0.0".into();
        empty.source_url = scan_url(&config, "sample", "2.0.0").unwrap().to_string();
        empty.raw = Some(json!({"findings": []}));
        store(&db, &empty).unwrap();
        let mut failed = empty.clone();
        failed.id = "failed".into();
        failed.version = "3.0.0".into();
        failed.source_url = scan_url(&config, "sample", "3.0.0").unwrap().to_string();
        failed.decision = "error".into();
        failed.raw = None;
        store(&db, &failed).unwrap();
        let summaries = load_summaries(&db, &config).unwrap();
        assert_eq!(
            summaries
                .iter()
                .find(|r| r.id == "empty")
                .unwrap()
                .finding_count,
            Some(0)
        );
        assert_eq!(
            summaries
                .iter()
                .find(|r| r.id == "failed")
                .unwrap()
                .finding_count,
            None
        );
        let first = page(&db, &config, "project", "sample", "1.2.3", 0, 1).unwrap();
        assert_eq!(first.total, 2);
        assert_eq!(first.findings.len(), 1);
        assert_eq!(first.findings[0].finding_id.as_deref(), Some("GHSA-test"));
        assert_eq!(first.findings[0].package.as_deref(), Some("example/module"));
        assert_eq!(first.findings[0].installed_version.as_deref(), Some("1.0"));
        assert_eq!(first.findings[0].fixed_version.as_deref(), Some("1.1"));
        assert_eq!(first.report.unwrap().decision, "unknown");
        assert_eq!(
            page(&db, &config, "project", "sample", "1.2.3", 99, 1)
                .unwrap()
                .page,
            1
        );
        let mut another = settings();
        another.extra.insert(
            "slaps_api_url".into(),
            "https://another.example.test".into(),
        );
        assert!(
            page(&db, &another, "project", "sample", "1.2.3", 0, 1)
                .unwrap()
                .report
                .is_none()
        );
        assert!(decode_scan(json!({"scanReport":null})).unwrap().is_none());
        assert!(decode_scan(json!({"scanReport":"not json"})).is_err());
    }
    #[test]
    #[ignore = "Requires an external response file from a real SLAPS API request"]
    fn actual_scan_response_is_stored_with_all_findings() {
        let response: Value = serde_json::from_slice(
            &std::fs::read(std::env::var("GATETREE_SECURITY_RESPONSE_FILE").unwrap()).unwrap(),
        )
        .unwrap();
        let raw = decode_scan(response.get("data").cloned().unwrap_or(response))
            .unwrap()
            .unwrap();
        let expected = raw.get("findings").and_then(Value::as_array).unwrap().len();
        let mut rows = Vec::new();
        findings(&raw, &mut rows);
        assert_eq!(rows.len(), expected);
        assert!(
            rows.iter()
                .all(|row| value(row, &["Advisory_Name", "CVE", "id"]).is_some())
        );
        let config = settings();
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Test','/tmp/test','scm','')",[]).unwrap();
        let report = SecurityReport {
            id: "real-report".into(),
            project_id: "project".into(),
            image_name: "sample".into(),
            version: "1.2.3".into(),
            realm: "TEST".into(),
            scanner: "anchore".into(),
            source_url: scan_url(&config, "sample", "1.2.3").unwrap().to_string(),
            decision: "unknown".into(),
            finding_count: None,
            scan_time: None,
            fetched_at: 1,
            error: None,
            raw: Some(raw),
        };
        store(&db, &report).unwrap();
        let stored = page(&db, &config, "project", "sample", "1.2.3", 0, 20).unwrap();
        assert_eq!(stored.total, expected as i64);
        assert!(
            stored
                .findings
                .iter()
                .all(|row| row.finding_id.is_some() && row.package.is_some())
        );
        println!(
            "Stored and paged {expected} real findings with their advisory identifiers and packages."
        );
    }
}
