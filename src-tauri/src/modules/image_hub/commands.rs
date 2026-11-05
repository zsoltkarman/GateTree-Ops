use super::{
    artifacts,
    config::{self, ImageHubConfig},
    model::{FolderInput, ImageHubSnapshot, ProjectInput, RepositoryInput},
    project_sync::{self, SyncItem, SyncReport},
    service,
};
use crate::modules::workspace::WorkspaceState;
use tauri::{Manager, State};

#[tauri::command]
pub fn open_image_hub_report(state: State<'_, WorkspaceState>, url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid report URL.")?;
    let settings = state
        .with_connection(config::load)
        .map_err(|e| e.to_string())?;
    let saved_repository_url = state
        .with_connection(|db| super::service::permits_repository_url(db, &parsed))
        .map_err(|e| e.to_string())?;
    if !settings.permits(&parsed) && !saved_repository_url {
        return Err("This URL is not allowed by Image Hub → Config.".into());
    }
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open")
        .arg(parsed.as_str())
        .status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", parsed.as_str()])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open")
        .arg(parsed.as_str())
        .status();
    if status
        .map_err(|e| format!("Could not open report: {e}"))?
        .success()
    {
        Ok(())
    } else {
        Err("The system browser could not open the report.".into())
    }
}

#[tauri::command]
pub fn save_image_hub_config(
    state: State<'_, WorkspaceState>,
    input: ImageHubConfig,
) -> Result<ImageHubConfig, String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Wait for repository sync to finish before changing configuration.")?;
    state
        .with_connection(|db| config::save(db, &input))
        .map_err(|e| e.to_string())?;
    Ok(input)
}

#[tauri::command]
pub fn import_image_hub_config(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<ImageHubConfig, String> {
    let length = std::fs::metadata(&path)
        .map_err(|_| "Cannot open the selected configuration file.")?
        .len();
    if length > 1024 * 1024 {
        return Err("Configuration file must be smaller than 1 MiB.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "Cannot read the selected configuration file.")?;
    let input: ImageHubConfig = serde_json::from_slice(&bytes)
        .map_err(|_| "The file is not a valid Image Hub configuration JSON file.")?;
    save_image_hub_config(state, input)
}

#[tauri::command]
pub fn save_image_hub_config_entry(
    state: State<'_, WorkspaceState>,
    original_name: Option<String>,
    name: String,
    value: String,
) -> Result<ImageHubConfig, String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Wait for project sync to finish before changing configuration.")?;
    state
        .with_connection(|db| config::save_entry(db, original_name.as_deref(), &name, &value))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_image_hub_config_entry(
    state: State<'_, WorkspaceState>,
    name: String,
) -> Result<ImageHubConfig, String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Wait for project sync to finish before changing configuration.")?;
    state
        .with_connection(|db| config::delete_entry(db, &name))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn refresh_image_hub_artifacts(
    app: tauri::AppHandle,
    id: String,
) -> Result<Vec<artifacts::LatestImage>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<WorkspaceState>();
        let snapshot = state
            .with_connection(|db| {
                service::project(db, &id)?;
                service::snapshot(db)
            })
            .map_err(|e| e.to_string())?;
        let repos = snapshot
            .repositories
            .into_iter()
            .filter(|r| r.project_id.as_deref() == Some(id.as_str()))
            .collect::<Vec<_>>();
        let raw_settings = snapshot.config;
        let settings = raw_settings.resolved()?;
        settings.validate()?;
        if !settings.artifacts_configured() { return Err("Configure artifact URLs in Image Hub → Config.".into()); }
        let images = artifacts::collect(&repos, &snapshot.latest_images, &settings)?;
        state
            .with_connection(|db| {
                service::project(db, &id)?;
                if config::load(db)? != raw_settings { return Err(rusqlite::Error::InvalidParameterName("Configuration changed during image collection. Open the project again to refresh.".into())); }
                let transaction = db.unchecked_transaction()?;
                for image in &images {
                    artifacts::store(&transaction, image)?;
                }
                transaction.commit()
            })
            .map_err(|e| e.to_string())?;
        Ok(images)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn save_image_hub_project(
    state: State<'_, WorkspaceState>,
    input: ProjectInput,
) -> Result<String, String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Wait for the current project sync to finish.")?;
    state
        .with_connection(|db| service::save_project(db, input))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_image_hub_project(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Wait for the current project sync to finish.")?;
    state
        .with_connection(|db| service::delete_project(db, &id))
        .map_err(|e| e.to_string())
}

pub(super) fn run_image_hub_sync(
    app: &tauri::AppHandle,
    id: &str,
    session: &str,
    tracker: &mut super::sync_job::Tracker<'_>,
) -> Result<SyncReport, String> {
    let _guard = project_sync::SYNC_LOCK
        .try_lock()
        .map_err(|_| "Another project sync is already running.")?;
    let state = app.state::<WorkspaceState>();
    let project = state
        .with_session_connection(session, |db| service::project(db, &id))
        .map_err(|e| e.to_string())?;
    let settings = state
        .with_session_connection(session, config::load)
        .map_err(|e| e.to_string())?;
    tracker.update("scm", 0, 0, 0, "running", "Discovering repositories")?;
    let mut repositories = project_sync::local_repositories(&project)?;
    let mut identities = repositories
        .iter()
        .map(|r| project_sync::remote_identity(&r.url))
        .collect::<Result<std::collections::HashSet<_>, _>>()?;
    let mut report = SyncReport {
        items: Vec::new(),
        remote_collected: false,
        remote_error: None,
        repository_link_summary: None,
        security_summary: None,
    };
    if project.source.is_empty() {
        report.remote_error = Some(
            "Project URL / ID is not configured. Only local repositories were collected.".into(),
        );
    } else {
        match project_sync::discover(&project, &settings) {
            Ok(remote) => {
                report.remote_collected = true;
                for repo in remote {
                    let identity = project_sync::remote_identity(&repo.url)?;
                    if identities.insert(identity.clone()) {
                        repositories.push(repo);
                    } else if let Some(local) = repositories.iter_mut().find(|r| {
                        project_sync::remote_identity(&r.url).ok().as_deref() == Some(&identity)
                    }) {
                        local.ocid = repo.ocid;
                    }
                }
            }
            Err(error) => {
                report.remote_error = Some(if super::security::authentication_required(&error) {
                    format!("{error} {}", super::security::auth_help(&settings))
                } else {
                    error
                })
            }
        }
    }
    let repository_total = repositories.len();
    tracker.update(
        "scm",
        0,
        repository_total,
        0,
        "running",
        report.remote_error.as_deref().unwrap_or(""),
    )?;
    for (index, repo) in repositories.into_iter().enumerate() {
        // A locked/switched workspace or edited/deleted project cannot receive stale results.
        let current = state
            .with_session_connection(session, |db| service::project(db, &id))
            .map_err(|e| e.to_string())?;
        if current.local_path != project.local_path
            || current.source != project.source
            || current.project_type != project.project_type
        {
            return Err("Project configuration changed. Run sync again.".into());
        }
        let path = std::path::Path::new(&project.local_path).join(&repo.relative_path);
        let identity = project_sync::remote_identity(&repo.url)?;
        let snapshot = state
            .with_session_connection(session, service::snapshot)
            .map_err(|e| e.to_string())?;
        let existing = snapshot.repositories.iter().find(|r| {
            r.project_id.as_deref() == Some(id)
                && project_sync::remote_identity(&r.url).ok().as_deref() == Some(&identity)
        });
        let existing_id = existing.map(|r| r.id.clone());
        let result = project_sync::ensure_local(&project, &repo);
        let (local_path, status, message) = match result {
            Ok((path, status)) => (path, status.to_owned(), String::new()),
            Err(error) => (path, "failed".to_owned(), error),
        };
        let mut input = project_sync::repository_input(&project, &repo, &local_path, existing_id);
        // A local-only sync or expired remote session must not erase collected metadata.
        if input.ocid.is_empty() {
            input.ocid = existing.map(|repo| repo.ocid.clone()).unwrap_or_default();
        }
        state
            .with_session_connection(session, |db| {
                service::project(db, &id)?;
                service::save_repository(db, input)
            })
            .map_err(|e| e.to_string())?;
        report.items.push(SyncItem {
            name: repo.name,
            path: local_path.to_string_lossy().into_owned(),
            status,
            message,
        });
        tracker.update(
            "scm",
            index + 1,
            repository_total,
            report
                .items
                .iter()
                .filter(|item| item.status == "failed")
                .count(),
            "running",
            report.remote_error.as_deref().unwrap_or(""),
        )?;
    }
    if project.project_type == "scm" {
        let snapshot = state
            .with_session_connection(session, service::snapshot)
            .map_err(|e| e.to_string())?;
        let repos = snapshot
            .repositories
            .iter()
            .filter(|repo| repo.project_id.as_deref() == Some(id))
            .collect::<Vec<_>>();
        let ready = repos
            .iter()
            .filter(|repo| repo.browser_url.is_some())
            .count();
        let missing_ids = repos.iter().filter(|repo| repo.ocid.is_empty()).count();
        let mut summary = format!(
            "Project-based repository links: {ready}/{} ready.",
            repos.len()
        );
        if ready < repos.len() {
            if !project.source.starts_with("https://") && !project.source.starts_with("http://") {
                summary.push_str(
                    " Save the full SCM project page URL in Edit project → Project URL / ID.",
                );
            }
            if missing_ids > 0 {
                summary.push_str(&format!(" {missing_ids} repositories still have no OCID; remote SCM discovery must succeed to collect them."));
            }
        }
        report.repository_link_summary = Some(summary);
    }
    let scm_errors = report
        .items
        .iter()
        .filter(|item| item.status == "failed")
        .count();
    tracker.update(
        "scm",
        repository_total,
        repository_total,
        scm_errors,
        if scm_errors > 0 || report.remote_error.is_some() {
            "error"
        } else {
            "synced"
        },
        report
            .remote_error
            .as_deref()
            .or_else(|| {
                report
                    .items
                    .iter()
                    .find(|item| item.status == "failed")
                    .map(|item| item.message.as_str())
            })
            .unwrap_or(""),
    )?;
    let runtime = settings.resolved()?;
    let snapshot = state
        .with_session_connection(session, service::snapshot)
        .map_err(|e| e.to_string())?;
    let repos = snapshot
        .repositories
        .into_iter()
        .filter(|repo| repo.project_id.as_deref() == Some(id))
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if runtime.artifacts_configured() {
        let mut af_done = 0;
        let mut errors = 0;
        tracker.update("af", 0, repos.len(), 0, "running", "")?;
        for chunk in repos.chunks(4) {
            let fresh = artifacts::collect(chunk, &[], &runtime)?;
            for image in fresh {
                if image.status == "error" {
                    errors += 1;
                }
                state
                    .with_session_connection(session, |db| {
                        service::project(db, &id)?;
                        artifacts::store(db, &image)
                    })
                    .map_err(|e| e.to_string())?;
                af_done += 1;
                tracker.update("af", af_done, repos.len(), errors, "running", "")?;
            }
        }
        tracker.update(
            "af",
            repos.len(),
            repos.len(),
            errors,
            if errors > 0 { "error" } else { "synced" },
            "",
        )?;
    } else {
        tracker.update(
            "af",
            0,
            0,
            0,
            "not_configured",
            "Configure artifact URLs in Image Hub → Config.",
        )?;
    }
    let images = state
        .with_session_connection(session, artifacts::load)
        .map_err(|e| e.to_string())?;
    let mut targets = std::collections::BTreeSet::new();
    for image in &images {
        if repos.iter().any(|repo| repo.id == image.repository_id)
            && matches!(image.status.as_str(), "found" | "error")
        {
            if let Some(version) = &image.version {
                targets.insert((image.image_name.clone(), version.clone()));
            }
        }
    }
    if let Some(directory) = runtime.extra.get("ansible_dir").filter(|v| !v.is_empty()) {
        match super::ansible::collect(directory) {
            Ok(images) => {
                for image in images {
                    if repos.iter().any(|repo| {
                        repo.name == image.image_name
                            || repo.name == image.role
                            || repo.name.replace('-', "_") == image.role
                    }) {
                        if let Some(version) = image.version {
                            targets.insert((image.image_name, version));
                        }
                    }
                }
            }
            Err(error) => warnings.push(error),
        }
    }
    let cached_reports = state
        .with_session_connection(session, |db| super::security::load_summaries(db, &runtime))
        .map_err(|e| e.to_string())?;
    match super::security::api_settings(&runtime) {
        Err(error) => {
            tracker.update("slaps", 0, targets.len(), 0, "not_configured", &error)?;
            report.security_summary = Some(error);
        }
        Ok((_, realm)) => {
            let mut downloaded = 0;
            let mut failed = 0;
            let mut attempted = 0;
            let target_count = targets.len();
            tracker.update("slaps", 0, target_count, 0, "running", "")?;
            for (image, version) in targets {
                let is_latest = images.iter().any(|item| {
                    item.image_name == image && item.version.as_deref() == Some(version.as_str())
                });
                if !is_latest {
                    if let Some(cached) = cached_reports.iter().find(|scan| {
                        scan.project_id == id
                            && scan.image_name == image
                            && scan.version == version
                            && scan.error.is_none()
                            && scan.decision != "pending"
                            && scan.decision != "error"
                            && scan.fetched_at <= artifacts::now()
                            && artifacts::now() - scan.fetched_at < 3600
                    }) {
                        state
                            .with_session_connection(session, |db| {
                                super::builds::link_security(db, id, &image, &version, &cached.id)
                            })
                            .map_err(|e| e.to_string())?;
                        attempted += 1;
                        downloaded += 1;
                        tracker.update("slaps", attempted, target_count, failed, "running", "")?;
                        continue;
                    }
                }
                let scan = super::security::fetch(&id, &image, &version, &runtime)?;
                attempted += 1;
                if scan.raw.is_some() {
                    downloaded += 1;
                }
                if let Some(error) = &scan.error {
                    failed += 1;
                    warnings.push(if super::security::authentication_required(error) {
                        format!(
                            "{image}:{version}: {error} {}",
                            super::security::auth_help(&runtime)
                        )
                    } else {
                        format!("{image}:{version}: {error}")
                    });
                }
                let auth_failed = scan
                    .error
                    .as_deref()
                    .is_some_and(super::security::authentication_required);
                state
                    .with_session_connection(session, |db| {
                        service::project(db, &id)?;
                        super::security::store(db, &scan)?;
                        if scan.raw.is_some() {
                            super::builds::link_security(db, id, &image, &version, &scan.id)?;
                        }
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
                tracker.update(
                    "slaps",
                    attempted,
                    target_count,
                    failed,
                    "running",
                    warnings.last().map(String::as_str).unwrap_or(""),
                )?;
                if auth_failed {
                    break;
                }
            }
            tracker.update(
                "slaps",
                attempted,
                target_count,
                failed,
                if failed > 0 || attempted < target_count {
                    "error"
                } else {
                    "synced"
                },
                warnings.last().map(String::as_str).unwrap_or(""),
            )?;
            report.security_summary = Some(format!(
                "SLAPS {realm}: {downloaded}/{target_count} reports downloaded and saved to SQL; {failed} failed; {} not attempted.{}",
                target_count - attempted,
                if warnings.is_empty() {
                    String::new()
                } else {
                    format!(" {}", warnings.join(" "))
                }
            ));
        }
    }
    Ok(report)
}

#[tauri::command]
pub fn get_image_hub(state: State<'_, WorkspaceState>) -> Result<ImageHubSnapshot, String> {
    state
        .with_connection(service::snapshot)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_image_hub_folder(
    state: State<'_, WorkspaceState>,
    input: FolderInput,
) -> Result<String, String> {
    state
        .with_connection(|db| service::save_folder(db, input))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_image_hub_repository(
    state: State<'_, WorkspaceState>,
    input: RepositoryInput,
) -> Result<String, String> {
    state
        .with_connection(|db| service::save_repository(db, input))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_image_hub_folder(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    state
        .with_connection(|db| service::delete_folder(db, &id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_image_hub_repository(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    state
        .with_connection(|db| service::delete_repository(db, &id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_hub_ansible_images(
    app: tauri::AppHandle,
) -> Result<Vec<super::ansible::AnsibleImage>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = app
            .state::<WorkspaceState>()
            .with_connection(config::load)
            .map_err(|e| e.to_string())?;
        super::ansible::collect(
            settings
                .extra
                .get("ansible_dir")
                .map(String::as_str)
                .unwrap_or(""),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_image_hub_security_report(
    state: State<'_, WorkspaceState>,
    repository_id: String,
    image_name: String,
    version: String,
    page: i64,
    page_size: i64,
) -> Result<super::security::ReportPage, String> {
    state.with_connection(|db| {
        let project_id: String = db.query_row("SELECT project_id FROM image_hub_repositories WHERE id=?1 AND project_id IS NOT NULL", [repository_id], |r| r.get(0))?;
        let config = config::load(db)?.resolved().map_err(rusqlite::Error::InvalidParameterName)?;
        super::security::page(db,&config,&project_id,&image_name,&version,page,page_size)
    }).map_err(|e| e.to_string())
}
