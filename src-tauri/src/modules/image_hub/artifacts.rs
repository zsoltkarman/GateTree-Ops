use super::{
    config::{self, ImageHubConfig},
    model::Repository,
    project_sync,
};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestImage {
    pub repository_id: String,
    pub image_name: String,
    pub version: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub checked_at: i64,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn load(db: &Connection) -> rusqlite::Result<Vec<LatestImage>> {
    let mut statement = db.prepare("SELECT repository_id, image_name, version, url, status, error, checked_at FROM image_hub_latest_images")?;
    statement
        .query_map([], |r| {
            Ok(LatestImage {
                repository_id: r.get(0)?,
                image_name: r.get(1)?,
                version: r.get(2)?,
                url: r.get(3)?,
                status: r.get(4)?,
                error: r.get(5)?,
                checked_at: r.get(6)?,
            })
        })?
        .collect()
}

pub fn store(db: &Connection, image: &LatestImage) -> rusqlite::Result<()> {
    db.execute("INSERT INTO image_hub_latest_images(repository_id, image_name, version, url, status, error, checked_at)
        SELECT id, ?2, ?3, ?4, ?5, ?6, ?7 FROM image_hub_repositories WHERE id=?1 AND name=?2
        ON CONFLICT(repository_id) DO UPDATE SET image_name=excluded.image_name, version=CASE WHEN excluded.status='error' THEN COALESCE(excluded.version,image_hub_latest_images.version) ELSE excluded.version END,
        url=CASE WHEN excluded.status='error' THEN COALESCE(excluded.url,image_hub_latest_images.url) ELSE excluded.url END, status=excluded.status, error=excluded.error, checked_at=excluded.checked_at",
        params![image.repository_id, image.image_name, image.version, image.url, image.status, image.error, image.checked_at])?;
    Ok(())
}

fn version_parts(tag: &str) -> Option<Vec<u64>> {
    let parts = tag.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    parts.into_iter().map(|p| p.parse().ok()).collect()
}

fn release_tags(folder: &Value) -> Result<Vec<String>, String> {
    let children = folder
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Artifactory did not return a folder listing.")?;
    let mut tags = children
        .iter()
        .filter(|item| item.get("folder").and_then(Value::as_bool) == Some(true))
        .filter_map(|item| item.get("uri").and_then(Value::as_str))
        .map(|uri| uri.trim_start_matches('/'))
        .filter_map(|tag| version_parts(tag).map(|parts| (parts, tag.to_owned())))
        .collect::<Vec<_>>();
    tags.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    Ok(tags.into_iter().map(|(_, tag)| tag).collect())
}

fn has_manifest(folder: &Value) -> Result<bool, String> {
    let children = folder
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Artifactory did not return an image folder listing.")?;
    Ok(children.iter().any(|item| {
        item.get("folder").and_then(Value::as_bool) == Some(false)
            && item
                .get("uri")
                .and_then(Value::as_str)
                .is_some_and(|uri| uri.trim_start_matches('/') == "manifest.json")
    }))
}

fn request(url: &str) -> Result<Option<Value>, String> {
    let output = project_sync::run(
        "curl",
        &[
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "20",
            "--header",
            "Accept: application/json",
            "--write-out",
            "\n%{http_code}",
            "--url",
            url,
        ],
        None,
    )?;
    let (body, status) = output
        .rsplit_once('\n')
        .ok_or("Artifactory returned no HTTP status.")?;
    match status.trim() {
        "200" => serde_json::from_str(body)
            .map(Some)
            .map_err(|_| "Artifactory returned invalid JSON.".into()),
        "404" => Ok(None),
        "401" | "403" => {
            Err("Artifactory access was denied. The image version was not collected.".into())
        }
        status => Err(format!("Artifactory returned HTTP {status}.")),
    }
}

fn lookup_with(
    name: &str,
    settings: &ImageHubConfig,
    get: &impl Fn(&str) -> Result<Option<Value>, String>,
) -> Result<Option<(String, String)>, String> {
    if !settings.artifacts_configured() {
        return Err("Configure artifact URLs in Image Hub → Config.".into());
    }
    let folder_url = config::expand(
        &settings.artifact_folder_url_template,
        settings,
        name,
        "",
        "",
    )?;
    let Some(folder) = get(&folder_url)? else {
        return Ok(None);
    };
    for tag in release_tags(&folder)? {
        // A tag folder is published only once its Docker manifest exists.
        let mut tag_url =
            url::Url::parse(&folder_url).map_err(|_| "Invalid artifact folder URL.")?;
        tag_url
            .path_segments_mut()
            .map_err(|_| "Invalid artifact folder URL.")?
            .pop_if_empty()
            .push(&tag);
        if let Some(folder) = get(tag_url.as_str())? {
            if has_manifest(&folder)? {
                return Ok(Some((
                    tag.clone(),
                    config::expand(
                        &settings.artifact_page_url_template,
                        settings,
                        name,
                        "",
                        &tag,
                    )?,
                )));
            }
        }
    }
    Ok(None)
}

pub fn lookup(repo: &Repository, settings: &ImageHubConfig) -> LatestImage {
    let mut image = LatestImage {
        repository_id: repo.id.clone(),
        image_name: repo.name.clone(),
        version: None,
        url: None,
        status: "not_found".into(),
        error: None,
        checked_at: now(),
    };
    match lookup_with(&repo.name, settings, &request) {
        Ok(Some((version, url))) => {
            image.status = "found".into();
            image.version = Some(version);
            image.url = Some(url);
        }
        Ok(None) => {}
        Err(error) => {
            image.status = "error".into();
            image.error = Some(error);
        }
    }
    image
}

pub fn collect(
    repos: &[Repository],
    cache: &[LatestImage],
    settings: &ImageHubConfig,
) -> Result<Vec<LatestImage>, String> {
    // Limit concurrency to four requests; cache each repository result for one minute.
    let current = now();
    std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|worker| {
                scope.spawn(move || {
                    repos
                        .iter()
                        .skip(worker)
                        .step_by(4)
                        .map(|repo| {
                            cache
                                .iter()
                                .find(|entry| {
                                    entry.repository_id == repo.id
                                        && entry.image_name == repo.name
                                        && entry.checked_at <= current
                                        && current - entry.checked_at < 60
                                })
                                .cloned()
                                .unwrap_or_else(|| lookup(repo, settings))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut result = Vec::new();
        for handle in handles {
            result.extend(
                handle
                    .join()
                    .map_err(|_| "An artifact lookup worker failed.".to_owned())?,
            );
        }
        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_af_refresh_preserves_the_last_known_version_but_records_the_error() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute(
            "INSERT INTO image_hub_repositories(id,name,repo_type) VALUES('repo','sample','scm')",
            [],
        )
        .unwrap();
        let mut image = LatestImage {
            repository_id: "repo".into(),
            image_name: "sample".into(),
            version: Some("1.2.3".into()),
            url: Some("https://artifacts.example.test/sample/1.2.3".into()),
            status: "found".into(),
            error: None,
            checked_at: 1,
        };
        store(&db, &image).unwrap();
        image.status = "error".into();
        image.version = None;
        image.url = None;
        image.error = Some("Network unavailable".into());
        image.checked_at = 2;
        store(&db, &image).unwrap();
        let loaded = load(&db).unwrap();
        assert_eq!(loaded[0].version.as_deref(), Some("1.2.3"));
        assert!(loaded[0].url.is_some());
        assert_eq!(loaded[0].status, "error");
        assert_eq!(loaded[0].checked_at, 2);
    }

    use serde_json::json;

    #[test]
    #[ignore = "Requires an explicit external configuration file and image name"]
    fn configured_artifact_lookup_live() {
        let path = std::env::var("GATETREE_IMAGE_HUB_TEST_CONFIG")
            .expect("Set the external configuration path");
        let image = std::env::var("GATETREE_IMAGE_HUB_TEST_IMAGE").expect("Set the image name");
        let settings: ImageHubConfig =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        settings.validate().unwrap();
        let settings = settings.resolved().unwrap();
        let (version, url) = lookup_with(&image, &settings, &request)
            .unwrap()
            .expect("A published version must exist");
        println!(
            "{}",
            serde_json::json!({ "image": image, "version": version, "url": url })
        );
    }
    #[test]
    fn latest_image_uses_numeric_versions_and_requires_a_manifest() {
        let settings = ImageHubConfig {
            artifact_folder_url_template:
                "https://artifacts.example.test/api/storage/{artifactRepository}/{repo}".into(),
            artifact_page_url_template:
                "https://artifacts.example.test/ui/repos/{artifactRepository}/{repo}/{version}"
                    .into(),
            artifact_repository: "releases".into(),
            ..ImageHubConfig::default()
        };
        let result = lookup_with("sample-service", &settings, &|path| Ok(Some(if path.ends_with("/sample-service") {
            json!({"children":[{"uri":"/2026.9.1","folder":true},{"uri":"/2026.10.7","folder":true},{"uri":"/2026.10.8","folder":true},{"uri":"/latest","folder":true},{"uri":"/2026.11.1-rc1","folder":true},{"uri":"/2027.1.1","folder":false}]})
        } else if path.ends_with("2026.10.8") { json!({"children":[]}) }
        else { json!({"children":[{"uri":"/manifest.json","folder":false}]}) }))).unwrap().unwrap();
        assert_eq!(result.0, "2026.10.7");
        assert!(result.1.ends_with("/sample-service/2026.10.7"));
        assert!(
            lookup_with("missing", &settings, &|_| Ok(None))
                .unwrap()
                .is_none()
        );
        assert!(lookup_with("denied", &settings, &|_| Err("HTTP 403".into())).is_err());
        assert!(release_tags(&json!({"errors":[]})).is_err());
    }

    #[test]
    fn latest_image_cache_persists_and_is_deleted_with_repository() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute(
            "INSERT INTO image_hub_repositories(id,name,repo_type) VALUES('repo','one','scm')",
            [],
        )
        .unwrap();
        let row = LatestImage {
            repository_id: "repo".into(),
            image_name: "one".into(),
            version: Some("2026.10.7".into()),
            url: Some("https://artifacts.example.test/ui/repos/releases/one/2026.10.7".into()),
            status: "found".into(),
            error: None,
            checked_at: now(),
        };
        store(&db, &row).unwrap();
        assert_eq!(load(&db).unwrap()[0].version.as_deref(), Some("2026.10.7"));
        db.execute("DELETE FROM image_hub_repositories WHERE id='repo'", [])
            .unwrap();
        assert!(load(&db).unwrap().is_empty());
    }
}
