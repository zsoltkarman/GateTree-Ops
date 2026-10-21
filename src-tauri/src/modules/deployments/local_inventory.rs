use crate::modules::workspace::WorkspaceState;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State};

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAnsibleSettings {
    pub path: Option<String>,
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("local-ansible.json"))
}
fn read_settings(path: &Path) -> Result<LocalAnsibleSettings, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Cannot read local Ansible settings: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LocalAnsibleSettings::default()),
        Err(e) => Err(e.to_string()),
    }
}
fn project_path(path: &str) -> Result<PathBuf, String> {
    let path =
        fs::canonicalize(path.trim()).map_err(|e| format!("Cannot open Ansible folder: {e}"))?;
    if !path.is_dir() || !path.join("inventory").is_dir() {
        return Err("Choose the Ansible project folder containing the inventory directory.".into());
    }
    Ok(path)
}
fn write_settings(file: &Path, path: &str) -> Result<LocalAnsibleSettings, String> {
    let settings = LocalAnsibleSettings {
        path: Some(project_path(path)?.to_string_lossy().into_owned()),
    };
    fs::create_dir_all(file.parent().ok_or("Invalid settings path")?).map_err(|e| e.to_string())?;
    let temporary = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // Windows does not replace existing files with rename; a settings write is small and recoverable.
    #[cfg(target_os = "windows")]
    if file.exists() {
        fs::remove_file(file).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, file).map_err(|e| e.to_string())?;
    Ok(settings)
}
#[tauri::command]
pub fn get_local_ansible_settings(app: tauri::AppHandle) -> Result<LocalAnsibleSettings, String> {
    read_settings(&settings_path(&app)?)
}
#[tauri::command]
pub fn set_local_ansible_folder(
    app: tauri::AppHandle,
    path: String,
) -> Result<LocalAnsibleSettings, String> {
    write_settings(&settings_path(&app)?, &path)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySnapshot {
    pub path: String,
    pub environment: String,
    pub read_at: u64,
    pub revision: String,
    pub sites: Vec<InventorySite>,
    pub warnings: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySite {
    pub name: String,
    pub group: Option<String>,
    pub hosts: Vec<InventoryHost>,
}
#[derive(Serialize)]
pub struct InventoryHost {
    pub name: String,
    pub ip: String,
    pub roles: Vec<String>,
}
#[derive(Default)]
struct Group {
    hosts: BTreeSet<String>,
    children: BTreeSet<String>,
}
fn parse_group(
    name: &str,
    value: &Value,
    groups: &mut BTreeMap<String, Group>,
    addresses: &mut BTreeMap<String, String>,
) {
    groups.entry(name.into()).or_default();
    if let Some(hosts) = value.get("hosts").and_then(Value::as_mapping) {
        for (host, vars) in hosts {
            if let Some(host) = host.as_str() {
                groups.get_mut(name).unwrap().hosts.insert(host.into());
                if let Some(ip) = vars
                    .get("ansible_host")
                    .or_else(|| vars.get("ansible_ssh_host"))
                    .and_then(Value::as_str)
                {
                    addresses.insert(host.into(), ip.into());
                }
            }
        }
    }
    if let Some(children) = value.get("children").and_then(Value::as_mapping) {
        for (child, definition) in children {
            if let Some(child) = child.as_str() {
                groups.get_mut(name).unwrap().children.insert(child.into());
                parse_group(child, definition, groups, addresses);
            }
        }
    }
}
fn members(
    name: &str,
    groups: &BTreeMap<String, Group>,
    visited: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    if !visited.insert(name.into()) {
        return BTreeSet::new();
    }
    let Some(group) = groups.get(name) else {
        return BTreeSet::new();
    };
    let mut result = group.hosts.clone();
    for child in &group.children {
        result.extend(members(child, groups, visited));
    }
    result
}
fn directories(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| format!("Cannot read inventory folder: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            result.push(entry.path());
        }
    }
    result.sort();
    Ok(result)
}
fn scan_inventory(project: &Path, environment: &str) -> Result<InventorySnapshot, String> {
    if environment.is_empty()
        || environment == "."
        || environment == ".."
        || environment.contains(['/', '\\'])
    {
        return Err("Process must define a valid deployment payload_defaults.ENVIRONMENT.".into());
    }
    let base = project.join("inventory");
    let base = if base.join("gbucs_orasys_host_inventory").is_dir() {
        base.join("gbucs_orasys_host_inventory")
    } else {
        base
    };
    let env_dir = base.join(environment);
    if fs::symlink_metadata(&env_dir)
        .map_err(|e| format!("Environment '{environment}' is unavailable: {e}"))?
        .file_type()
        .is_symlink()
    {
        return Err("Inventory environment must be a local directory, not a symlink.".into());
    }
    let mut sites = Vec::new();
    let mut warnings = Vec::new();
    let mut hash = Sha256::new();
    let mut total = 0;
    for site in directories(&env_dir)? {
        if sites.len() >= 1000 {
            return Err("Too many inventory sites.".into());
        }
        let name = site.file_name().unwrap().to_string_lossy().into_owned();
        let mut groups = BTreeMap::new();
        let mut addresses = BTreeMap::new();
        let mut role_names = BTreeSet::new();
        let mut files = Vec::new();
        for entry in fs::read_dir(&site).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if entry.file_type().map_err(|e| e.to_string())?.is_file()
                && matches!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("yaml" | "yml")
                )
            {
                files.push(path);
            }
        }
        files.sort();
        for file in files {
            let length = fs::metadata(&file).map_err(|e| e.to_string())?.len();
            total += length;
            if length > 2_000_000 || total > 32_000_000 {
                return Err("Inventory exceeds the supported size.".into());
            }
            let bytes = fs::read(&file).map_err(|e| e.to_string())?;
            hash.update(
                file.strip_prefix(project)
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes(),
            );
            hash.update([0]);
            hash.update(&bytes);
            hash.update([0]);
            let value: Value = serde_yaml::from_slice(&bytes)
                .map_err(|_| format!("Invalid YAML in {}", file.display()))?;
            let Some(mapping) = value.as_mapping() else {
                continue;
            };
            for (group, definition) in mapping {
                if let Some(group) = group.as_str() {
                    parse_group(group, definition, &mut groups, &mut addresses);
                    if file.file_stem().and_then(|s| s.to_str()) == Some("roles") {
                        role_names.insert(group.to_string());
                    }
                }
            }
        }
        if groups.is_empty() {
            continue;
        }
        let expected = format!("{environment}_{name}");
        let group = groups.contains_key(&expected).then_some(expected);
        if group.is_none() {
            warnings.push(format!(
                "{name}: no environment/site group found; select explicit hosts for this site."
            ));
        }
        let env_hosts = members(environment, &groups, &mut BTreeSet::new());
        let site_hosts = group
            .as_ref()
            .map(|g| members(g, &groups, &mut BTreeSet::new()));
        let role_hosts: BTreeMap<_, _> = role_names
            .iter()
            .map(|role| (role.clone(), members(role, &groups, &mut BTreeSet::new())))
            .collect();
        let mut hosts = Vec::new();
        for host in env_hosts {
            if site_hosts
                .as_ref()
                .is_some_and(|site| !site.contains(&host))
            {
                continue;
            }
            let roles = role_hosts
                .iter()
                .filter(|(_, hs)| hs.contains(&host))
                .map(|(role, _)| role.clone())
                .collect();
            hosts.push(InventoryHost {
                ip: addresses.get(&host).cloned().unwrap_or_default(),
                name: host,
                roles,
            });
        }
        if hosts.is_empty() {
            warnings.push(format!(
                "{name}: no hosts match the environment/site groups."
            ));
        }
        sites.push(InventorySite { name, group, hosts });
    }
    Ok(InventorySnapshot {
        path: project.to_string_lossy().into_owned(),
        environment: environment.into(),
        read_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        revision: format!("{:x}", hash.finalize()),
        sites,
        warnings,
    })
}
#[tauri::command]
pub async fn get_local_deployment_inventory(
    app: tauri::AppHandle,
    state: State<'_, WorkspaceState>,
    process_id: String,
) -> Result<InventorySnapshot, String> {
    let path = read_settings(&settings_path(&app)?)?
        .path
        .ok_or("Choose a local Ansible folder first.")?;
    let definition: String = state
        .with_connection(|db| {
            db.query_row(
                "SELECT definition_json FROM deployment_processes WHERE id = ?1",
                [&process_id],
                |row| row.get(0),
            )
        })
        .map_err(|e| e.to_string())?;
    let process: serde_json::Value =
        serde_json::from_str(&definition).map_err(|e| e.to_string())?;
    let environment = process
        .pointer("/details/container/payload_defaults/ENVIRONMENT")
        .or_else(|| process.pointer("/details/olam/payload_defaults/ENVIRONMENT"))
        .and_then(serde_json::Value::as_str)
        .ok_or("Process must define container.payload_defaults.ENVIRONMENT or olam.payload_defaults.ENVIRONMENT.")?
        .to_string();
    tauri::async_runtime::spawn_blocking(move || {
        scan_inventory(&project_path(&path)?, &environment)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_settings_survive_restart_and_reject_invalid_projects() {
        let temp = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(temp.join("project/inventory")).unwrap();
        let file = temp.join("settings/local-ansible.json");
        assert!(read_settings(&file).unwrap().path.is_none());
        let saved = write_settings(&file, temp.join("project").to_str().unwrap()).unwrap();
        assert_eq!(read_settings(&file).unwrap().path, saved.path);
        assert!(write_settings(&file, temp.to_str().unwrap()).is_err());
        assert_eq!(read_settings(&file).unwrap().path, saved.path);
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn inventory_intersects_environment_site_role_and_handles_children() {
        let temp = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let site = temp.join("inventory/gbucs_orasys_host_inventory/test/site1");
        fs::create_dir_all(&site).unwrap();
        fs::write(site.join("hosts.yaml"), "all:\n  hosts:\n    host1:\n      ansible_host: 192.0.2.1\n    host2:\n      ansible_ssh_host: 192.0.2.2\n").unwrap();
        fs::write(
            site.join("environment.yaml"),
            "test:\n  hosts:\n    host1:\n    host2:\n",
        )
        .unwrap();
        fs::write(
            site.join("locations.yaml"),
            "test_site1:\n  hosts:\n    host1:\n",
        )
        .unwrap();
        fs::write(
            site.join("roles.yaml"),
            "service:\n  children:\n    child:\n      hosts:\n        host1:\n        host2:\n",
        )
        .unwrap();
        let snap = scan_inventory(&temp, "test").unwrap();
        assert_eq!(snap.sites[0].hosts.len(), 1);
        assert_eq!(snap.sites[0].hosts[0].ip, "192.0.2.1");
        assert_eq!(snap.sites[0].hosts[0].roles, vec!["service"]);
        assert!(scan_inventory(&temp, "../test").is_err());
        let rev = snap.revision;
        fs::write(
            site.join("locations.yaml"),
            "test_site1:\n  hosts:\n    host2:\n",
        )
        .unwrap();
        assert_ne!(scan_inventory(&temp, "test").unwrap().revision, rev);
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    #[ignore = "Explicit local inventory diagnostic"]
    fn local_inventory_diagnostic() {
        let path = std::env::var("GATETREE_OPS_ANSIBLE_TEST_PATH").unwrap();
        let snap = scan_inventory(Path::new(&path), "ohai").unwrap();
        let count: usize = snap
            .sites
            .iter()
            .filter(|s| ["kc1", "ls3"].contains(&s.name.as_str()))
            .map(|s| {
                s.hosts
                    .iter()
                    .filter(|h| h.roles.contains(&"log_decorator".into()))
                    .count()
            })
            .sum();
        assert_eq!(count, 4);
        println!("Verified log_decorator: KC1 + LS3 = {count} hosts.");
    }
}
