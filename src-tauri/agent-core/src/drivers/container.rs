//! Local container Ansible runner. Process config owns the image and playbooks;
//! Planner owns the targets/account. Passwords never become command arguments.
use super::DriverOutcome;
use crate::{AgentCredential, AgentJobStatus};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Container config requires {field}."))
}
fn relative_file(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':', '\n', '\r'])
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
pub(crate) fn validate_config(config: &Value) -> Result<(), String> {
    let image = text(config, "image")?;
    if image.starts_with('-') || image.contains(char::is_whitespace) {
        return Err("Container image must be one image reference.".into());
    }
    for field in ["engine", "repository"] {
        if config.get(field).is_some_and(|value| !value.is_string()) {
            return Err(format!("Container {field} must be text."));
        }
    }
    if !matches!(
        config
            .get("engine")
            .and_then(Value::as_str)
            .unwrap_or("auto"),
        "auto" | "docker" | "podman"
    ) {
        return Err("Container engine must be auto, docker or podman.".into());
    }
    if config
        .get("repository")
        .and_then(Value::as_str)
        .unwrap_or("local_ansible_folder")
        != "local_ansible_folder"
    {
        return Err("Container repository must be local_ansible_folder.".into());
    }
    for field in [
        "deployment_playbook",
        "validation_playbook",
        "ansible_config",
    ] {
        if let Some(value) = config.get(field) {
            if !value.as_str().is_some_and(relative_file) {
                return Err(format!(
                    "Container {field} must be a relative project file path."
                ));
            }
        } else if field == "deployment_playbook" {
            return Err("Container deployment_playbook is required.".into());
        }
    }
    if !identifier(
        config
            .pointer("/payload_defaults/ENVIRONMENT")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ) {
        return Err("Container payload_defaults.ENVIRONMENT is required and must be a simple environment name.".into());
    }
    if let Some(value) = config.get("role_variable") {
        if !value.as_str().is_some_and(|s| {
            !s.is_empty()
                && s.bytes().enumerate().all(|(i, c)| {
                    c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                })
        }) {
            return Err("Container role_variable must be a variable name.".into());
        }
    }
    if let Some(value) = config.get("vault_password_file") {
        if !value.as_str().is_some_and(|s| {
            (s.starts_with('/') || s.starts_with("~/")) && !s.contains(['\n', '\r', '\0'])
        }) {
            return Err("Container vault_password_file must be an absolute path or ~/ path inside the image.".into());
        }
    }
    if let Some(value) = config.get("timeout_seconds") {
        if !value.as_u64().is_some_and(|n| n > 0 && n <= 86400) {
            return Err("Container timeout_seconds must be between 1 and 86400.".into());
        }
    }
    Ok(())
}

struct PrivateFiles(PathBuf);
impl PrivateFiles {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("gatetree-ansible-{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|e| format!("Cannot prepare container files: {e}"))?;
        Ok(Self(path))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.0.join(name)).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())
    }
}
impl Drop for PrivateFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Output {
    success: bool,
    text: String,
}
// Capture output to private files so a full stdout/stderr pipe cannot deadlock.
// Engine commands can be cancelled even while an image is being pulled.
fn command(engine: &Path, args: &[String], cancel: &dyn Fn() -> bool) -> Result<Output, String> {
    if cancel() {
        return Err("Container execution stopped by the operator.".into());
    }
    let files = PrivateFiles::new()?;
    files.write("output", b"")?;
    let output = OpenOptions::new()
        .write(true)
        .open(files.0.join("output"))
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(engine)
        .args(args)
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output)
        .spawn()
        .map_err(|e| format!("Cannot run container engine: {e}"))?;
    let started = Instant::now();
    let command_timeout = Duration::from_secs(if args.first().is_some_and(|s| s == "create") {
        300
    } else {
        30
    });
    let status = loop {
        if cancel() || started.elapsed() > command_timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Container engine operation stopped or timed out.".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(Output {
        success: status.success(),
        text: String::from_utf8_lossy(&fs::read(files.0.join("output")).unwrap_or_default())
            .into_owned(),
    })
}
fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
fn engine_candidates(kind: &str) -> Vec<PathBuf> {
    let names = if kind == "auto" {
        vec!["docker", "podman"]
    } else {
        vec![kind]
    };
    let mut dirs =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .iter()
            .map(PathBuf::from),
    );
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".rd/bin"));
    }
    names
        .into_iter()
        .flat_map(|name| {
            dirs.iter().map(move |dir| {
                dir.join(if cfg!(windows) {
                    format!("{name}.exe")
                } else {
                    name.to_string()
                })
            })
        })
        .collect()
}
fn engine(config: &Value, cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    for path in engine_candidates(config["engine"].as_str().unwrap_or("auto")) {
        if cancel() {
            return Err("Container execution stopped.".into());
        }
        if path.is_file() && command(&path, &args(&["info"]), cancel).is_ok_and(|o| o.success) {
            return Ok(path);
        }
    }
    Err("No running Docker/Podman engine was found. Start the container engine and retry.".into())
}
struct ContainerGuard {
    engine: PathBuf,
    name: String,
}
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = command(&self.engine, &args(&["rm", "-f", &self.name]), &|| false);
    }
}

pub(crate) fn stop_orphan(process: &str, job: &AgentJobStatus) {
    let Some(id) = job.deployment_job_id.or(job.validation_job_id) else {
        return;
    };
    let Ok(process) = serde_json::from_str::<Value>(process) else {
        return;
    };
    let config = &process["details"]["container"];
    let name = format!("gatetree-ops-ansible-{id}");
    for candidate in engine_candidates(config["engine"].as_str().unwrap_or("auto")) {
        if candidate.is_file() {
            let _ = command(&candidate, &args(&["rm", "-f", &name]), &|| false);
        }
    }
}

const VALIDATION_PLAYBOOK: &str = r#"- name: Validate approved deployment targets
  hosts: "{{ _gatetree_target_pattern }}"
  gather_facts: false
  any_errors_fatal: true
  tasks:
    - name: Validate SSH connectivity
      ansible.builtin.ping:
    - name: Show deployment target
      ansible.builtin.debug:
        msg:
          - "Role name: {{ _gatetree_role }}"
          - "Environment: {{ ENVIRONMENT }}"
          - "Ansible hostname: {{ inventory_hostname }}"
          - "Ansible ssh host: {{ ansible_host | default(ansible_ssh_host | default(inventory_hostname)) }}"
          - "IMAGE_VERSION: {{ IMAGE_VERSION | default('n/a') }}"
"#;
// Shell syntax is fixed. Paths/arguments remain separate positional parameters;
// ~/ is resolved against the image's HOME, never the operator's home directory.
const ENTRYPOINT: &str = r#"vault=$1; shift
case "$vault" in '~/'*) vault="$HOME/${vault#\~/}";; esac
trap 'rm -f /run/gatetree/ssh-password /run/gatetree/extra-vars.json' EXIT
for kind in group_vars host_vars; do
    if [ -d "/workspace/playbooks/$kind" ]; then
        ln -s "/workspace/playbooks/$kind" "/run/gatetree/$kind" || exit 2
    fi
done
if [ ! -r "$vault" ]; then printf 'Ansible Vault password file is unavailable inside the image.\n' >&2; exit 2; fi
export ANSIBLE_NOCOLOR=1 ANSIBLE_FORCE_COLOR=0 PYTHONUNBUFFERED=1
ansible-playbook "$@" --vault-password-file "$vault"
"#;

pub fn execute(
    process: &str,
    plan: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_started: &dyn Fn(&str, i64),
    on_output: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<DriverOutcome, String> {
    let process: Value =
        serde_json::from_str(process).map_err(|e| format!("Invalid container Process: {e}"))?;
    let config = &process["details"]["container"];
    validate_config(config)?;
    let engine = engine(config, cancel)?;
    execute_with_engine(
        &engine,
        config,
        plan,
        job,
        credentials,
        on_started,
        on_output,
        cancel,
    )
}
fn execute_with_engine(
    engine: &Path,
    config: &Value,
    plan: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_started: &dyn Fn(&str, i64),
    on_output: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<DriverOutcome, String> {
    let plan: Value =
        serde_yaml::from_str(plan).map_err(|e| format!("Invalid Ansible Plan: {e}"))?;
    let inputs = plan
        .pointer("/inputs/ansible")
        .or_else(|| plan.pointer("/inputs/container"))
        .or_else(|| plan.pointer("/inputs/olam"))
        .ok_or("Plan inputs.ansible is required.")?;
    let account = inputs["account"]
        .as_str()
        .ok_or("Plan account is required.")?;
    let credential = credentials
        .iter()
        .find(|c| c.reference.eq_ignore_ascii_case(account))
        .ok_or_else(|| format!("Unlock KeePass to resolve deployment account '{account}'."))?;
    if credential.username.trim().is_empty() {
        return Err("Deployment account username is empty.".into());
    }
    let root = fs::canonicalize(text(config, "repository_path")?)
        .map_err(|e| format!("Cannot read Local Ansible folder: {e}"))?;
    if root.to_string_lossy().contains(',') {
        return Err("Local Ansible folder must not contain a comma for Docker bind mounts.".into());
    }
    let environment = text(&config["payload_defaults"], "ENVIRONMENT")?;
    let mut payload: Value = serde_json::from_str(
        job.payload
            .as_deref()
            .ok_or("Container payload is missing.")?,
    )
    .map_err(|e| e.to_string())?;
    let role = payload["playbook_name"]
        .as_str()
        .filter(|s| identifier(s))
        .ok_or("Container role must be a simple role name.")?
        .to_owned();
    let limits = payload["ansible_limit"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("Container target list is empty.")?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| {
                    !s.is_empty()
                        && !s.starts_with('-')
                        && s.bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                })
                .map(str::to_owned)
                .ok_or("Container targets must be explicit host or inventory group names.")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sites = payload["_container_sites"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("Select at least one site in Planner for container execution.")?
        .iter()
        .map(|s| {
            s.as_str()
                .filter(|s| identifier(s))
                .map(str::to_owned)
                .ok_or("Container site must be a simple name.")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let deployment = job.action == "deploy"
        || (job.action == "validate_deploy" && job.phase.as_deref() == Some("deployment"));
    let phase = if deployment {
        "deployment"
    } else {
        "validation"
    };
    let playbook = if deployment {
        Some(text(config, "deployment_playbook")?)
    } else {
        config["validation_playbook"].as_str()
    };
    if let Some(playbook) = playbook {
        let canonical = fs::canonicalize(root.join(playbook))
            .map_err(|e| format!("Cannot read {phase} playbook: {e}"))?;
        if !canonical.starts_with(&root) {
            return Err("Container playbook must stay inside Local Ansible folder.".into());
        }
    }
    let files = PrivateFiles::new()?;
    files.write("ssh-password", credential.password.as_bytes())?;
    let object = payload
        .as_object_mut()
        .ok_or("Container payload must be an object.")?;
    for field in [
        "ansible_password",
        "ansible_become_pass",
        "_container_sites",
    ] {
        object.remove(field);
    }
    object.insert("ansible_user".into(), json!(credential.username));
    object.insert("ENVIRONMENT".into(), json!(environment));
    object.insert(
        config["role_variable"]
            .as_str()
            .unwrap_or("ROLE_NAME")
            .into(),
        json!(role),
    );
    object.insert("_gatetree_role".into(), json!(role));
    object.insert(
        "_gatetree_target_pattern".into(),
        json!(format!("{}:&{role}:&{environment}", limits.join(":"))),
    );
    files.write(
        "extra-vars.json",
        &serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
    )?;
    if playbook.is_none() {
        files.write("validation.yml", VALIDATION_PLAYBOOK.as_bytes())?;
    }
    let id = (u64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap())
        & ((1u64 << 53) - 1))
        .max(1) as i64;
    let name = format!("gatetree-ops-ansible-{id}");
    let guard = ContainerGuard {
        engine: engine.to_owned(),
        name: name.clone(),
    };
    let mut create = args(&[
        "create",
        "--pull=missing",
        "--name",
        &name,
        "--workdir",
        "/workspace",
        "--mount",
        &format!("type=bind,src={},dst=/workspace,readonly", root.display()),
        "--entrypoint",
        "/bin/sh",
    ]);
    if let Some(config_path) = config["ansible_config"].as_str() {
        if !root.join(config_path).is_file() {
            return Err("Configured ansible_config file was not found.".into());
        }
        create.extend(args(&[
            "-e",
            &format!("ANSIBLE_CONFIG=/workspace/{config_path}"),
        ]));
    }
    create.extend(args(&[
        text(config, "image")?,
        "-c",
        ENTRYPOINT,
        "gatetree-ansible",
        config["vault_password_file"]
            .as_str()
            .unwrap_or("~/.orasys_vault_pass"),
        &playbook
            .map(|p| format!("/workspace/{p}"))
            .unwrap_or_else(|| "/run/gatetree/validation.yml".into()),
    ]));
    for site in sites {
        let nested = root
            .join("inventory/gbucs_orasys_host_inventory")
            .join(environment)
            .join(&site);
        let relative = if nested.is_dir() {
            format!("inventory/gbucs_orasys_host_inventory/{environment}/{site}")
        } else {
            format!("inventory/{environment}/{site}")
        };
        let canonical = fs::canonicalize(root.join(&relative))
            .map_err(|e| format!("Site inventory was not found: {e}"))?;
        if !canonical.starts_with(&root) {
            return Err("Site inventory must stay inside Local Ansible folder.".into());
        }
        create.extend(args(&["-i", &format!("/workspace/{relative}")]));
    }
    create.extend(args(&[
        "-e",
        "@/run/gatetree/extra-vars.json",
        "--user",
        &credential.username,
        "--become",
        "--become-pass-file=/run/gatetree/ssh-password",
        "--conn-pass-file=/run/gatetree/ssh-password",
    ]));
    let redact = |output: &str| {
        if credential.password.is_empty() {
            output.to_owned()
        } else {
            output.replace(&credential.password, "[REDACTED]")
        }
    };
    let checked = |args: &[String]| -> Result<String, String> {
        let output = command(engine, args, cancel)?;
        if output.success {
            Ok(output.text)
        } else {
            Err(format!("Container engine failed: {}", redact(&output.text)))
        }
    };
    // Persist the name/ID before create/pull so an Agent restart can clean up
    // even a container interrupted during preparation.
    on_started(phase, id);
    checked(&create)?;
    checked(&args(&[
        "cp",
        &files.0.to_string_lossy(),
        &format!("{name}:/run/gatetree"),
    ]))?;
    drop(files);
    checked(&args(&["start", &name]))?;
    let started = Instant::now();
    let timeout = Duration::from_secs(config["timeout_seconds"].as_u64().unwrap_or(7200));
    let mut previous = String::new();
    loop {
        if cancel() {
            return Err("Container deployment stopped by the operator.".into());
        }
        if started.elapsed() > timeout {
            return Err(format!("{previous}\nContainer execution timed out."));
        }
        let logs =
            checked(&args(&["logs", &name])).map_err(|error| format!("{previous}\n{error}"))?;
        let output = redact(&logs);
        if output != previous {
            on_output(&output);
            previous = output;
        }
        let state = checked(&args(&["inspect", "--format", "{{json .State}}", &name]))
            .map_err(|error| format!("{previous}\n{error}"))?;
        let state: Value = serde_json::from_str(state.trim())
            .map_err(|e| format!("{previous}\nInvalid container state: {e}"))?;
        if matches!(state["Status"].as_str(), Some("exited" | "dead")) {
            // Capture final bytes after observing exit; logs can grow between reads.
            let output = redact(
                &checked(&args(&["logs", &name]))
                    .map_err(|error| format!("{previous}\n{error}"))?,
            );
            on_output(&output);
            let log = format!(
                "Container {phase} Job #{id}, image {}.\n{output}",
                text(config, "image")?
            );
            if state["ExitCode"].as_i64() != Some(0) {
                return Err(format!(
                    "{log}\nAnsible failed (exit code {}).",
                    state["ExitCode"]
                ));
            }
            if output.to_lowercase().contains("no hosts matched") {
                return Ok(DriverOutcome::Warning(format!(
                    "{log}\nNo hosts matched; Plan stopped."
                )));
            }
            drop(guard);
            return Ok(if job.action == "validate_deploy" && !deployment {
                DriverOutcome::AwaitingApproval(format!(
                    "{log}\nValidation passed. Approve to deploy to the same targets."
                ))
            } else {
                DriverOutcome::Completed(log)
            });
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_has_no_embedded_image_or_vault_account_requirement() {
        let mut config = json!({"image":"registry.test/deployer:2026.9.3","deployment_playbook":"playbooks/deploy.yml","payload_defaults":{"ENVIRONMENT":"devcorp"}});
        validate_config(&config).unwrap();
        config["image"] = json!("registry.test/deployer:next");
        validate_config(&config).unwrap();
        for field in [
            "deployment_playbook",
            "ansible_config",
            "validation_playbook",
        ] {
            let mut invalid = config.clone();
            invalid[field] = json!("../outside.yml");
            assert!(validate_config(&invalid).is_err());
        }
        config["engine"] = json!("sh -c bad");
        assert!(validate_config(&config).is_err());
    }
    #[cfg(unix)]
    fn fixture() -> (
        PrivateFiles,
        PathBuf,
        Value,
        AgentJobStatus,
        Vec<AgentCredential>,
    ) {
        use std::os::unix::fs::PermissionsExt;
        let root = PrivateFiles::new().unwrap();
        fs::create_dir_all(root.0.join("inventory/devcorp/phx")).unwrap();
        fs::create_dir(root.0.join("playbooks")).unwrap();
        fs::write(root.0.join("playbooks/deploy.yml"), "---\n").unwrap();
        let engine = root.0.join("fake-engine");
        fs::write(&engine,format!(r#"#!/usr/bin/env python3
import json,sys,pathlib,shutil
root=pathlib.Path({root})
a=sys.argv[1:]
with (root/'calls').open('a') as f:f.write(json.dumps(a)+'\n')
if a[0]=='create': print('container-created')
elif a[0]=='cp': shutil.copytree(a[1],root/'copied')
elif a[0]=='start': print('container-started')
elif a[0]=='logs': print('no hosts matched' if (root/'no-hosts').exists() else ('working test-secret' if not (root/'inspected').exists() else 'finished'))
elif a[0]=='inspect':
 n=int((root/'inspected').read_text()) if (root/'inspected').exists() else 0
 (root/'inspected').write_text(str(n+1))
 print(json.dumps({{'Status':'running' if n==0 else 'exited','ExitCode':9 if (root/'fail').exists() else 0}}))
elif a[0]=='rm': (root/'removed').touch()
"#,root=serde_json::to_string(&root.0.to_string_lossy()).unwrap())).unwrap();
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
        let config = json!({"image":"registry.test/deployer:chosen-version","repository_path":root.0,"deployment_playbook":"playbooks/deploy.yml","payload_defaults":{"ENVIRONMENT":"devcorp"}});
        let job:AgentJobStatus=serde_json::from_value(json!({"id":"job","sequence":1,"name":"Deploy","driver":"container","action":"validate_deploy","target":"host-1","details":"Validation pending","payload":json!({"playbook_name":"apache_kafka","STRATEGY":"linear","ansible_limit":["host-1"],"_container_sites":["phx"],"ansible_user":"$encrypted$","ansible_password":"$encrypted$","ansible_become_pass":"$encrypted$","VERSION":"003"}).to_string(),"status":"running","log":"","phase":"validation"})).unwrap();
        let credentials = vec![AgentCredential {
            reference: "deploy-account".into(),
            username: "deployer".into(),
            password: "test-secret".into(),
        }];
        (root, engine, config, job, credentials)
    }
    #[cfg(unix)]
    #[test]
    fn validation_streams_before_approval_and_uses_image_vault_without_password_arguments() {
        let (root, engine, config, job, credentials) = fixture();
        let outputs = std::cell::RefCell::new(Vec::new());
        let ids = std::cell::RefCell::new(Vec::new());
        let result = execute_with_engine(
            &engine,
            &config,
            "inputs:\n  ansible:\n    account: deploy-account",
            &job,
            &credentials,
            &|phase, id| ids.borrow_mut().push((phase.to_string(), id)),
            &|s| outputs.borrow_mut().push(s.to_owned()),
            &|| false,
        )
        .unwrap();
        assert!(matches!(result, DriverOutcome::AwaitingApproval(_)));
        assert_eq!(ids.borrow()[0].0, "validation");
        assert!(outputs.borrow().len() >= 2);
        assert!(outputs.borrow()[0].contains("working [REDACTED]"));
        let calls = fs::read_to_string(root.0.join("calls")).unwrap();
        assert!(!calls.contains("test-secret"));
        assert!(calls.contains("registry.test/deployer:chosen-version"));
        assert!(calls.contains("~/.orasys_vault_pass"));
        assert!(!calls.contains("ANSIBLE_CONFIG="));
        let vars: Value =
            serde_json::from_slice(&fs::read(root.0.join("copied/extra-vars.json")).unwrap())
                .unwrap();
        assert_eq!(vars["ROLE_NAME"], "apache_kafka");
        assert_eq!(vars["VERSION"], "003");
        assert_eq!(vars["ansible_user"], "deployer");
        assert!(vars.get("ansible_password").is_none());
        assert!(root.0.join("removed").exists());
        let commands = calls
            .lines()
            .map(|line| serde_json::from_str::<Vec<String>>(line).unwrap())
            .collect::<Vec<_>>();
        let cp = commands.iter().find(|a| a[0] == "cp").unwrap();
        assert!(!Path::new(&cp[1]).exists());
    }
    #[cfg(unix)]
    #[test]
    fn approved_phase_deploys_and_nonzero_exit_preserves_logs() {
        let (root, engine, config, mut job, credentials) = fixture();
        job.phase = Some("deployment".into());
        fs::write(root.0.join("fail"), "").unwrap();
        let result = execute_with_engine(
            &engine,
            &config,
            "inputs:\n  ansible:\n    account: deploy-account",
            &job,
            &credentials,
            &|phase, _| assert_eq!(phase, "deployment"),
            &|_| {},
            &|| false,
        );
        let error = result.err().unwrap();
        assert!(error.contains("finished"));
        assert!(error.contains("exit code 9"));
        assert!(root.0.join("removed").exists());
        let calls = fs::read_to_string(root.0.join("calls")).unwrap();
        assert!(calls.contains("/workspace/playbooks/deploy.yml"));
    }
    #[cfg(unix)]
    #[test]
    fn cancellation_removes_running_container() {
        let (root, engine, config, job, credentials) = fixture();
        let cancelled = std::cell::Cell::new(false);
        let result = execute_with_engine(
            &engine,
            &config,
            "inputs:\n  ansible:\n    account: deploy-account",
            &job,
            &credentials,
            &|_, _| {},
            &|_| cancelled.set(true),
            &|| cancelled.get(),
        );
        assert!(result.is_err());
        assert!(root.0.join("removed").exists());
    }
    #[cfg(unix)]
    #[test]
    fn missing_sites_prevents_any_container_launch() {
        let (root, engine, config, mut job, credentials) = fixture();
        let mut payload: Value = serde_json::from_str(job.payload.as_ref().unwrap()).unwrap();
        payload["_container_sites"] = json!([]);
        job.payload = Some(payload.to_string());
        assert!(
            execute_with_engine(
                &engine,
                &config,
                "inputs:\n  ansible:\n    account: deploy-account",
                &job,
                &credentials,
                &|_, _| {},
                &|_| {},
                &|| false
            )
            .is_err()
        );
        assert!(!root.0.join("calls").exists());
    }
    #[cfg(unix)]
    #[test]
    fn empty_inventory_stops_before_operator_approval() {
        let (root, engine, config, job, credentials) = fixture();
        fs::write(root.0.join("no-hosts"), "").unwrap();
        let result = execute_with_engine(
            &engine,
            &config,
            "inputs:\n  ansible:\n    account: deploy-account",
            &job,
            &credentials,
            &|_, _| {},
            &|_| {},
            &|| false,
        )
        .unwrap();
        assert!(matches!(result, DriverOutcome::Warning(_)));
        assert!(root.0.join("removed").exists());
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "Explicit local-image smoke test: synthetic localhost inventory only"]
    fn local_image_runs_validation_then_synthetic_deployment() {
        let image =
            std::env::var("GATETREE_OPS_CONTAINER_TEST_IMAGE").expect("Set local image reference");
        let (root, _fake, mut config, mut job, mut credentials) = fixture();
        fs::write(root.0.join("inventory/devcorp/phx/hosts.yml"),"all:\n  children:\n    devcorp:\n      children:\n        apache_kafka:\n          hosts:\n            localhost:\n              ansible_connection: local\n").unwrap();
        fs::create_dir(root.0.join("playbooks/group_vars")).unwrap();
        fs::write(
            root.0.join("playbooks/group_vars/devcorp.yml"),
            "IMAGE_VERSION: smoke-preview\nmarker: from-project-vars\n",
        )
        .unwrap();
        fs::write(root.0.join("playbooks/deploy.yml"),"- hosts: \"{{ _gatetree_target_pattern }}\"\n  gather_facts: false\n  tasks:\n    - ansible.builtin.debug:\n        msg: \"Synthetic localhost only {{ ROLE_NAME }} {{ marker }}\"\n").unwrap();
        config["image"] = json!(image);
        config["engine"] = json!("docker");
        credentials[0].username = "root".into();
        let mut payload: Value = serde_json::from_str(job.payload.as_ref().unwrap()).unwrap();
        payload["ansible_limit"] = json!(["localhost"]);
        job.payload = Some(payload.to_string());
        let plan = "inputs:\n  ansible:\n    account: deploy-account";
        let validation = execute(
            &json!({"details":{"container":config}}).to_string(),
            plan,
            &job,
            &credentials,
            &|_, _| {},
            &|_| {},
            &|| false,
        )
        .unwrap();
        match validation {
            DriverOutcome::AwaitingApproval(log) => {
                assert!(log.contains("localhost"));
                assert!(log.contains("smoke-preview"));
            }
            _ => panic!("Validation must await approval"),
        }
        job.phase = Some("deployment".into());
        match execute(
            &json!({"details":{"container":config}}).to_string(),
            plan,
            &job,
            &credentials,
            &|_, _| {},
            &|_| {},
            &|| false,
        )
        .unwrap()
        {
            DriverOutcome::Completed(log) => {
                assert!(log.contains("Synthetic localhost only apache_kafka from-project-vars"))
            }
            _ => panic!("Synthetic deployment must complete"),
        }
    }
}
