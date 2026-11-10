use super::{
    config::ImageHubConfig,
    model::{Project, RepositoryInput},
    service,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

pub static SYNC_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct DiscoveredRepository {
    pub name: String,
    pub relative_path: PathBuf,
    pub url: String,
    pub ocid: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncItem {
    pub name: String,
    pub path: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub items: Vec<SyncItem>,
    pub remote_collected: bool,
    pub remote_error: Option<String>,
    pub repository_link_summary: Option<String>,
    pub security_summary: Option<String>,
}

impl SyncReport {
    pub fn scm_detail(&self) -> String {
        let mut details = Vec::new();
        if let Some(error) = &self.remote_error {
            details.push(format!("Repository discovery: {error}"));
        }
        for item in self.items.iter().filter(|item| item.status == "failed") {
            details.push(format!("{}: {}", item.name, item.message));
        }
        details.join("\n")
    }
}

fn executable(name: &str) -> PathBuf {
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join(&filename);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    #[cfg(target_os = "macos")]
    for path in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let candidate = Path::new(path).join(&filename);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(filename)
}

fn oci_failure(args: &[&str], stderr: &[u8], exit: i32) -> String {
    let text = String::from_utf8_lossy(stderr);
    let parsed = text
        .find('{')
        .zip(text.rfind('}'))
        .and_then(|(start, end)| serde_json::from_str::<Value>(&text[start..=end]).ok());
    let status = parsed
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_u64);
    let code = parsed
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .filter(|code| {
            code.len() <= 80 && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        });
    let option = |key| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1])
            .unwrap_or("OCI CLI default")
    };
    let detail = match code {
        Some("NotAuthorizedOrNotFound") => {
            "The selected identity cannot access this project, or the project/region is incorrect. Check scm_profile and scm_auth in Image Hub → Config."
        }
        _ => {
            "Check the selected OCI profile, authentication, region and project access in Image Hub → Config."
        }
    };
    format!(
        "OCI request failed ({}{}; exit {exit}; profile: {}; auth: {}). {detail}",
        code.unwrap_or("no service error code"),
        status
            .map(|status| format!("; HTTP {status}"))
            .unwrap_or_default(),
        option("--profile"),
        option("--auth")
    )
}

fn command_bytes(
    program: &str,
    args: &[&str],
    success: bool,
    exit: i32,
    output: Vec<u8>,
    stderr: &[u8],
) -> Result<Vec<u8>, String> {
    if !success {
        if program == "oci" {
            if [output.as_slice(), stderr].iter().any(|bytes| {
                super::security::authentication_required(&String::from_utf8_lossy(bytes))
            }) {
                let profile = args
                    .windows(2)
                    .find(|pair| pair[0] == "--profile")
                    .map(|pair| pair[1])
                    .unwrap_or("OCI CLI default");
                return Err(format!(
                    "OCI_AUTH_REQUIRED: OCI CLI authentication failed (profile: {profile})."
                ));
            }
            return Err(oci_failure(args, stderr, exit));
        }
        return Err(format!(
            "{program} failed (exit {exit}). Check its existing authentication, repository access and SSH host trust in a terminal, then retry."
        ));
    }
    if output.len() >= 32 * 1024 * 1024 {
        return Err(format!(
            "{program} returned too much data; narrow the project scope."
        ));
    }
    Ok(output)
}
fn command_output(
    program: &str,
    args: &[&str],
    success: bool,
    exit: i32,
    output: Vec<u8>,
    stderr: &[u8],
) -> Result<String, String> {
    String::from_utf8(command_bytes(program, args, success, exit, output, stderr)?)
        .map_err(|_| format!("{program} returned invalid UTF-8."))
}
pub(super) fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    command_output(program, args, true, 0, run_bytes(program, args, cwd)?, &[])
}

// No shell, no credential reads, and no interactive login in the GUI process.
pub(super) fn run_bytes(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable(program));
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GLAB_CHECK_UPDATE", "false")
        .env("GCM_INTERACTIVE", "Never")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=20 -o ServerAliveInterval=15 -o ServerAliveCountMax=2",
        );
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    let mut child = command.spawn().map_err(|_| {
        format!("Cannot start {program}. Install it and make it available on PATH.")
    })?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    fn drain(mut input: impl Read) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        while let Ok(count) = input.read(&mut buffer) {
            if count == 0 {
                break;
            }
            if bytes.len() < 32 * 1024 * 1024 {
                bytes.extend_from_slice(&buffer[..count]);
            }
        }
        bytes
    }
    let out = thread::spawn(move || drain(stdout));
    let err = thread::spawn(move || drain(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < Duration::from_secs(180) => {
                thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{program} timed out after 180 seconds. Check connectivity and authentication, then retry."
        ));
    };
    command_bytes(
        program,
        args,
        status.success(),
        status.code().unwrap_or(-1),
        output,
        &stderr,
    )
}

fn safe_relative(path: &str) -> Result<PathBuf, String> {
    let value = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || !value
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Repository path must stay inside the project's local directory.".into());
    }
    Ok(value.to_path_buf())
}

pub fn remote_identity(value: &str) -> Result<String, String> {
    let rewritten;
    let value = if service::is_ssh_shorthand(value) {
        let (user_host, path) = value.split_once(':').unwrap();
        rewritten = format!("ssh://{user_host}/{path}");
        &rewritten
    } else {
        value
    };
    let url = url::Url::parse(value).map_err(|_| "Invalid repository URL.".to_owned())?;
    if !["http", "https", "ssh", "git"].contains(&url.scheme())
        || url.host_str().is_none()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use a repository URL without passwords, query strings or fragments.".into());
    }
    if ["http", "https"].contains(&url.scheme()) && !url.username().is_empty() {
        return Err("Do not put credentials in repository URLs. Configure the Git credential helper instead.".into());
    }
    let mut host = url.host_str().unwrap().to_ascii_lowercase();
    if let Some(port) = url.port() {
        if !(url.scheme() == "ssh" && port == 22) {
            host.push_str(&format!(":{port}"));
        }
    }
    // SSH and HTTPS aliases for the same host/path have the same identity.
    let path = url.path().trim_matches('/').trim_end_matches(".git");
    if path.is_empty() {
        return Err("Repository URL has no repository path.".into());
    }
    Ok(format!("{host}/{path}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn gitlab_source(source: &str) -> Result<(String, String), String> {
    let url = url::Url::parse(source)
        .map_err(|_| "Enter the GitLab/CloudLab group URL in Project URL / ID.".to_owned())?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an HTTP(S) group URL without credentials, query strings or fragments.".into(),
        );
    }
    let path = url.path().trim_matches('/').trim_start_matches("groups/");
    if path.is_empty() || path.contains("/-/") {
        return Err("Use the group URL, without a settings or other /-/ suffix.".into());
    }
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap()),
        None => url.host_str().unwrap().to_owned(),
    };
    Ok((host, path.to_owned()))
}

fn gitlab_repositories(output: &str, group: &str) -> Result<Vec<DiscoveredRepository>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    // glab --paginate --output ndjson emits individual JSON records.
    for value in serde_json::Deserializer::from_str(output).into_iter::<Value>() {
        let value = value.map_err(|_| "GitLab returned an invalid repository list.".to_owned())?;
        let values = match value {
            Value::Array(values) => values,
            value => vec![value],
        };
        for value in values {
            let full_path = text(&value, "path_with_namespace");
            let relative = full_path
                .strip_prefix(&format!("{group}/"))
                .ok_or("GitLab returned a repository outside the selected group.")?;
            let url = text(&value, "ssh_url_to_repo");
            let url = if url.is_empty() {
                text(&value, "http_url_to_repo")
            } else {
                url
            };
            let identity = remote_identity(url)?;
            if seen.insert(identity) {
                result.push(DiscoveredRepository {
                    name: text(&value, "path").to_owned(),
                    relative_path: safe_relative(relative)?,
                    url: url.to_owned(),
                    ocid: String::new(),
                });
            }
        }
    }
    Ok(result)
}

pub(super) fn scm_source(source: &str) -> Result<(String, Option<String>), String> {
    if source.starts_with("ocid1.devopsproject.") {
        return Ok((source.to_owned(), None));
    }
    let url = url::Url::parse(source)
        .map_err(|_| "Enter the SCM project OCID or project URL.".to_owned())?;
    if url.host_str().is_none()
        || !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Use the SCM project OCID or an HTTP(S) project URL without credentials.".into(),
        );
    }
    let id = url
        .path_segments()
        .and_then(|mut parts| parts.find(|part| part.starts_with("ocid1.devopsproject.")))
        .ok_or("The URL does not contain a DevOps project OCID.")?;
    let region = url
        .query_pairs()
        .find(|(key, _)| key == "region")
        .map(|(_, value)| value.into_owned())
        .or_else(|| {
            url.query_pairs()
                .find(|(key, _)| key == "_ctx")
                .and_then(|(_, value)| {
                    value
                        .split(',')
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                })
        });
    Ok((id.to_owned(), region))
}

/// The console origin and context come from the user-supplied project URL.
/// Clone URLs remain separate so repository sync continues to use the Git endpoint.
pub(super) fn scm_repository_browser_url(source: &str, repository_id: &str) -> Option<String> {
    scm_source(source).ok()?;
    if !repository_id.starts_with("ocid1.devopsrepository.")
        || !repository_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return None;
    }
    let mut url = url::Url::parse(source).ok()?;
    if url.fragment().is_some() {
        return None;
    }
    let context = url
        .query_pairs()
        .filter(|(key, _)| key == "_ctx" || key == "region")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_path(&format!(
        "/devops-coderepository/repositories/{repository_id}"
    ));
    url.set_query(None);
    if !context.is_empty() {
        url.query_pairs_mut().extend_pairs(context);
    }
    Some(url.to_string())
}

fn scm_repositories(output: &str) -> Result<Vec<DiscoveredRepository>, String> {
    let value: Value = serde_json::from_str(output)
        .map_err(|_| "OCI returned invalid repository JSON.".to_owned())?;
    let rows = value
        .pointer("/data/items")
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .ok_or("OCI did not return a repository list.")?;
    let mut result = Vec::new();
    for item in rows {
        let url = text(item, "ssh-url");
        let url = if url.is_empty() {
            text(item, "http-url")
        } else {
            url
        };
        remote_identity(url)?;
        let name = text(item, "name");
        result.push(DiscoveredRepository {
            name: name.to_owned(),
            relative_path: safe_relative(name)?,
            url: url.to_owned(),
            ocid: text(item, "id").to_owned(),
        });
    }
    Ok(result)
}

pub fn discover(
    project: &Project,
    settings: &ImageHubConfig,
) -> Result<Vec<DiscoveredRepository>, String> {
    if project.source.trim().is_empty() {
        return Ok(Vec::new());
    }
    if project.project_type == "scm" {
        let settings = settings.resolved()?;
        let (id, region) = scm_source(&project.source)?;
        let mut args = vec![
            "devops",
            "repository",
            "list",
            "--project-id",
            &id,
            "--lifecycle-state",
            "ACTIVE",
            "--all",
            "--output",
            "json",
        ];
        if !settings.scm_profile.is_empty() {
            args.extend(["--profile", &settings.scm_profile]);
        }
        if !settings.scm_auth.is_empty() {
            args.extend(["--auth", &settings.scm_auth]);
        }
        let region = region
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!settings.region.is_empty()).then_some(settings.region.as_str()));
        if let Some(region) = region {
            args.extend(["--region", region]);
        }
        scm_repositories(&run("oci", &args, None)?)
    } else {
        let (host, group) = gitlab_source(&project.source)?;
        let encoded: String = url::form_urlencoded::byte_serialize(group.as_bytes()).collect();
        let endpoint = format!(
            "groups/{encoded}/projects?include_subgroups=true&with_shared=false&per_page=100"
        );
        gitlab_repositories(
            &run(
                "glab",
                &[
                    "api",
                    &endpoint,
                    "--hostname",
                    &host,
                    "--method",
                    "GET",
                    "--paginate",
                    "--output",
                    "ndjson",
                ],
                None,
            )?,
            &group,
        )
    }
}

pub fn local_repositories(project: &Project) -> Result<Vec<DiscoveredRepository>, String> {
    let base = Path::new(&project.local_path);
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    fn walk(
        base: &Path,
        path: &Path,
        depth: usize,
        result: &mut Vec<DiscoveredRepository>,
    ) -> Result<(), String> {
        if depth > 32 {
            return Err("Project directory nesting exceeds 32 levels.".into());
        }
        if path.join(".git").exists() {
            let url = run("git", &["remote", "get-url", "origin"], Some(path))?
                .trim()
                .to_owned();
            remote_identity(&url)?;
            let relative = path
                .strip_prefix(base)
                .map_err(|_| "Repository is outside the project folder.")?
                .to_path_buf();
            result.push(DiscoveredRepository {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                relative_path: relative,
                url,
                ocid: String::new(),
            });
            return Ok(());
        }
        for entry in
            fs::read_dir(path).map_err(|_| format!("Cannot read directory {}.", path.display()))?
        {
            let entry = entry.map_err(|_| "Cannot read project directory entry.")?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || ["node_modules", "target", "vendor"].contains(&name.as_ref())
            {
                continue;
            }
            if entry
                .file_type()
                .map_err(|_| "Cannot inspect project directory entry.")?
                .is_dir()
            {
                walk(base, &entry.path(), depth + 1, result)?;
            }
        }
        Ok(())
    }
    walk(base, base, 0, &mut result)?;
    Ok(result)
}

pub fn ensure_local(
    project: &Project,
    repo: &DiscoveredRepository,
) -> Result<(PathBuf, &'static str), String> {
    ensure_local_with(project, repo, &run)
}

fn ensure_local_with(
    project: &Project,
    repo: &DiscoveredRepository,
    execute: &impl Fn(&str, &[&str], Option<&Path>) -> Result<String, String>,
) -> Result<(PathBuf, &'static str), String> {
    let root = Path::new(&project.local_path);
    fs::create_dir_all(root).map_err(|_| "Cannot create the project's local directory.")?;
    let root =
        fs::canonicalize(root).map_err(|_| "Cannot resolve the project's local directory.")?;
    let relative = if repo.relative_path.as_os_str().is_empty() {
        PathBuf::new()
    } else {
        safe_relative(&repo.relative_path.to_string_lossy())?
    };
    let destination = root.join(relative);
    let identity = remote_identity(&repo.url)?;
    if destination.exists() {
        let canonical = fs::canonicalize(&destination)
            .map_err(|_| "Cannot resolve existing repository path.")?;
        if !canonical.starts_with(&root) {
            return Err("Repository path escapes the project directory through a symlink.".into());
        }
        if !destination.join(".git").exists() {
            return Err("The destination already exists and is not a Git repository. It was left unchanged.".into());
        }
        let top = execute("git", &["rev-parse", "--show-toplevel"], Some(&destination))?;
        if fs::canonicalize(top.trim()).map_err(|_| "Cannot resolve Git working tree.")?
            != canonical
        {
            return Err("The destination is not a repository root.".into());
        }
        let origin = execute("git", &["remote", "get-url", "origin"], Some(&destination))?;
        if remote_identity(origin.trim())? != identity {
            return Err(
                "A different repository already occupies this path. It was left unchanged.".into(),
            );
        }
        return Ok((destination, "existing"));
    }
    let parent = destination
        .parent()
        .ok_or("Invalid repository destination.")?;
    // Check existing ancestors before creating subgroup directories.
    let mut ancestor = parent;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("Invalid parent directory.")?;
    }
    if !fs::canonicalize(ancestor)
        .map_err(|_| "Cannot resolve parent directory.")?
        .starts_with(&root)
    {
        return Err("Repository path escapes the project directory through a symlink.".into());
    }
    fs::create_dir_all(parent).map_err(|_| "Cannot create subgroup directory.")?;
    let target = destination
        .to_str()
        .ok_or("Repository path is not valid UTF-8.")?;
    execute("git", &["clone", "--", &repo.url, target], Some(&root))?;
    Ok((destination, "cloned"))
}

pub fn repository_input(
    project: &Project,
    repo: &DiscoveredRepository,
    local_path: &Path,
    id: Option<String>,
) -> RepositoryInput {
    RepositoryInput {
        id,
        project_id: Some(project.id.clone()),
        folder_id: project.folder_id.clone(),
        name: repo.name.clone(),
        local_path: local_path.to_string_lossy().into_owned(),
        ocid: repo.ocid.clone(),
        url: repo.url.clone(),
        repo_type: project.project_type.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scm_details_include_every_failed_repository_and_discovery_error() {
        let mut report = SyncReport {
            items: Vec::new(),
            remote_collected: false,
            remote_error: None,
            repository_link_summary: None,
            security_summary: None,
        };
        assert_eq!(report.scm_detail(), "");
        for (name, status, message) in [
            ("existing", "existing", ""),
            ("first", "failed", "git failed (exit 128)"),
            ("second", "failed", "Destination is occupied"),
            ("third", "failed", "Cannot create directory"),
            ("new", "cloned", ""),
        ] {
            report.items.push(SyncItem {
                name: name.into(),
                path: String::new(),
                status: status.into(),
                message: message.into(),
            });
        }
        let failures = "first: git failed (exit 128)\nsecond: Destination is occupied\nthird: Cannot create directory";
        assert_eq!(report.scm_detail(), failures);
        report.remote_error = Some("Discovery failed".into());
        assert_eq!(
            report.scm_detail(),
            format!("Repository discovery: Discovery failed\n{failures}")
        );
    }

    #[test]
    fn successful_security_reports_are_never_classified_as_expired_cli_sessions() {
        let args = ["raw-request", "--profile", "TEST_SESSION"];
        let payload=br#"{"status":"200 OK","data":{"scanReport":{"findings":[{"details":"authentication failed; token has expired; security_token_file"}]}}}"#.to_vec();
        let result = command_output(
            "oci",
            &args,
            true,
            0,
            payload.clone(),
            b"warning mentioning security_token_file",
        )
        .unwrap();
        assert_eq!(result, String::from_utf8(payload).unwrap());
        let expired = command_output(
            "oci",
            &args,
            false,
            1,
            b"ERROR: This CLI session has expired".to_vec(),
            b"Abort",
        )
        .unwrap_err();
        assert!(expired.contains("OCI_AUTH_REQUIRED"));
        assert!(expired.contains("TEST_SESSION"));
        let denied = command_output(
            "oci",
            &args,
            false,
            1,
            vec![],
            br#"ServiceError: {"code":"NotAuthorizedOrNotFound"}"#,
        )
        .unwrap_err();
        assert!(!super::super::security::authentication_required(&denied));
    }
    #[test]
    fn oci_access_errors_identify_the_selected_profile_without_dumping_stderr() {
        let error = oci_failure(&["--profile", "TEST_SESSION", "--auth", "security_token"], b"ServiceError:\n{\"code\":\"NotAuthorizedOrNotFound\",\"private_detail\":\"do-not-display\"}", 1);
        assert!(error.contains("NotAuthorizedOrNotFound"));
        assert!(error.contains("profile: TEST_SESSION"));
        assert!(error.contains("auth: security_token"));
        assert!(error.contains("scm_profile"));
        assert!(!error.contains("SSH"));
        assert!(!error.contains("do-not-display"));
        assert!(oci_failure(&[], b"not json", 1).contains("OCI CLI default"));
    }

    #[test]
    fn repository_browser_link_uses_only_the_supplied_project_origin_and_context() {
        let source = "https://console.example.test/devops-project/projects/ocid1.devopsproject.oc1.test.project/overview?_ctx=region-a%2Caccount-a&tab=settings";
        let id = "ocid1.devopsrepository.oc1.test.repository";
        let url = scm_repository_browser_url(source, id).unwrap();
        assert_eq!(
            url,
            "https://console.example.test/devops-coderepository/repositories/ocid1.devopsrepository.oc1.test.repository?_ctx=region-a%2Caccount-a"
        );
        assert_eq!(scm_source(source).unwrap().1.as_deref(), Some("region-a"));
        assert!(scm_repository_browser_url("ocid1.devopsproject.oc1.test.project", id).is_none());
        assert!(scm_repository_browser_url(source, "").is_none());
        assert!(scm_repository_browser_url(source, "ocid1.devopsrepository.bad/path").is_none());
        assert!(
            scm_repository_browser_url(
                "https://user:password@console.example.test/ocid1.devopsproject.test",
                id
            )
            .is_none()
        );
    }

    #[test]
    fn clones_only_missing_repositories_and_preserves_existing_files() {
        let directory =
            std::env::temp_dir().join(format!("image-hub-clone-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        let project = Project {
            id: "test".into(),
            folder_id: None,
            name: "Team".into(),
            local_path: directory.to_string_lossy().into_owned(),
            project_type: "gitlab".into(),
            source: String::new(),
        };
        let repo = DiscoveredRepository {
            name: "repo".into(),
            relative_path: PathBuf::from("sub/repo"),
            url: "git@git.example.com:team/sub/repo.git".into(),
            ocid: String::new(),
        };
        let calls = std::cell::RefCell::new(Vec::new());
        let execute =
            |program: &str, args: &[&str], cwd: Option<&Path>| -> Result<String, String> {
                assert_eq!(program, "git");
                calls.borrow_mut().push(args[0].to_owned());
                match args[0] {
                    "clone" => {
                        assert_eq!(args[1], "--");
                        assert_eq!(args[2], repo.url);
                        fs::create_dir_all(Path::new(args[3]).join(".git")).unwrap();
                        Ok(String::new())
                    }
                    "rev-parse" => Ok(cwd.unwrap().to_string_lossy().into_owned()),
                    "remote" => Ok(repo.url.clone()),
                    _ => panic!("Unexpected Git write"),
                }
            };
        let (path, status) = ensure_local_with(&project, &repo, &execute).unwrap();
        assert_eq!(status, "cloned");
        fs::write(path.join("dirty.txt"), "unsaved local work").unwrap();
        assert_eq!(
            ensure_local_with(&project, &repo, &execute).unwrap().1,
            "existing"
        );
        assert_eq!(calls.borrow().iter().filter(|s| *s == "clone").count(), 1);
        assert_eq!(
            fs::read_to_string(path.join("dirty.txt")).unwrap(),
            "unsaved local work"
        );
        let mismatch = DiscoveredRepository {
            url: "git@git.example.com:other/repo.git".into(),
            ..repo.clone()
        };
        assert!(ensure_local_with(&project, &mismatch, &execute).is_err());
        let occupied = DiscoveredRepository {
            relative_path: PathBuf::from("occupied"),
            ..repo.clone()
        };
        fs::create_dir(directory.join("occupied")).unwrap();
        assert!(ensure_local_with(&project, &occupied, &execute).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(directory.parent().unwrap(), directory.join("escape"))
                .unwrap();
            let escaped = DiscoveredRepository {
                relative_path: PathBuf::from("escape/other"),
                ..repo.clone()
            };
            assert!(ensure_local_with(&project, &escaped, &execute).is_err());
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_parsing_preserves_explicit_hosts_and_region() {
        assert_eq!(
            gitlab_source("https://git.example.com/groups/team/sub").unwrap(),
            ("git.example.com".into(), "team/sub".into())
        );
        assert!(gitlab_source("https://git.example.com/team/-/settings").is_err());
        let (id, region) = scm_source("https://console.example.test/projects/ocid1.devopsproject.test.sample?region=test-region").unwrap();
        assert_eq!(id, "ocid1.devopsproject.test.sample");
        assert_eq!(region.as_deref(), Some("test-region"));
    }
    #[test]
    fn parses_paginated_gitlab_and_oci_outputs_and_preserves_subgroups() {
        let output = "{\"path\":\"one\",\"path_with_namespace\":\"team/sub/one\",\"ssh_url_to_repo\":\"git@git.example.com:team/sub/one.git\"}\n{\"path\":\"two\",\"path_with_namespace\":\"team/two\",\"http_url_to_repo\":\"https://git.example.com/team/two.git\"}";
        let repos = gitlab_repositories(output, "team").unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].relative_path, Path::new("sub/one"));
        assert!(gitlab_repositories(output, "another-team").is_err());
        let oci = r#"{"data":{"items":[{"name":"one","id":"ocid1.devopsrepository.test","ssh-url":"ssh://git@git.example.com/one"}]}}"#;
        assert_eq!(
            scm_repositories(oci).unwrap()[0].ocid,
            "ocid1.devopsrepository.test"
        );
        assert!(safe_relative("../outside").is_err());
        assert!(safe_relative("/absolute").is_err());
        assert!(remote_identity("https://token@git.example.com/secret").is_err());
        assert_eq!(
            remote_identity("git@git.example.com:a/b.git").unwrap(),
            remote_identity("https://git.example.com/a/b.git").unwrap()
        );
    }
}
