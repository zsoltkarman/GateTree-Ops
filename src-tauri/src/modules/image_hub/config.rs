use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageHubConfig {
    pub artifact_folder_url_template: String,
    pub artifact_page_url_template: String,
    pub artifact_repository: String,
    pub build_logs_url_template: String,
    pub slaps_url_template: String,
    pub repository_url_template: String,
    pub team: String,
    pub region: String,
    pub context: String,
    pub scm_profile: String,
    pub scm_auth: String,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, String>,
}

pub fn expand(
    template: &str,
    config: &ImageHubConfig,
    repo: &str,
    ocid: &str,
    version: &str,
) -> Result<String, String> {
    let values = [
        ("repo", repo),
        ("image", repo),
        ("version", version),
        ("ocid", ocid),
        ("team", config.team.as_str()),
        ("region", config.region.as_str()),
        ("context", config.context.as_str()),
        ("artifactRepository", config.artifact_repository.as_str()),
    ];
    let mut result = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        result.push_str(&remaining[..start]);
        let tail = &remaining[start + 1..];
        let end = tail.find('}').ok_or("Unclosed URL template placeholder.")?;
        let key = &tail[..end];
        let value = values
            .iter()
            .find(|(name, _)| *name == key)
            .ok_or_else(|| format!("Unknown URL placeholder: {{{key}}}"))?
            .1;
        result.push_str(
            &url::form_urlencoded::byte_serialize(value.as_bytes())
                .collect::<String>()
                .replace('+', "%20"),
        );
        remaining = &tail[end + 1..];
    }
    result.push_str(remaining);
    if result.contains('}') {
        return Err("Unexpected closing brace in URL template.".into());
    }
    Ok(result)
}

impl ImageHubConfig {
    pub fn resolved(&self) -> Result<Self, String> {
        fn base(value: &str) -> Result<url::Url, String> {
            let url =
                url::Url::parse(value).map_err(|_| "Enter an absolute HTTP(S) URL.".to_owned())?;
            if !["http", "https"].contains(&url.scheme())
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(
                    "Configuration URLs must use HTTP(S), without credentials or fragments.".into(),
                );
            }
            Ok(url)
        }
        fn append(url: &url::Url, suffix: &str) -> String {
            let mut result = format!(
                "{}{}{}",
                url.origin().ascii_serialization(),
                url.path().trim_end_matches('/'),
                suffix
            );
            if let Some(query) = url.query() {
                result.push('?');
                result.push_str(query);
            }
            result
        }
        let mut result = self.clone();
        if let Some(value) = self.extra.get("scm_profile") {
            result.scm_profile = value.trim().to_owned();
        }
        if let Some(value) = self.extra.get("scm_auth") {
            result.scm_auth = value.trim().to_owned();
        }
        if let Some(value) = self
            .extra
            .get("build_log_url")
            .filter(|value| !value.is_empty())
        {
            result.build_logs_url_template = append(&base(value)?, "/{repo}");
        }
        if let Some(value) = self
            .extra
            .get("security_scan_url")
            .filter(|value| !value.is_empty())
        {
            let mut url = base(value)?;
            let query = url
                .query_pairs()
                .filter(|(key, _)| key != "artifact" && key != "limit")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            url.set_query(None);
            url.query_pairs_mut()
                .extend_pairs(query)
                .append_pair("artifact", "IMAGEHUB_REPO_VALUE")
                .append_pair("limit", "15");
            result.slaps_url_template = url.as_str().replace("IMAGEHUB_REPO_VALUE", "{repo}");
        }
        if let Some(value) = self
            .extra
            .get("artifactory_url")
            .filter(|value| !value.is_empty())
        {
            let url = base(value)?;
            result.artifact_page_url_template = append(&url, "/{repo}/{version}");
            let marker = "/ui/repos/tree/General/";
            if !url.path().contains(marker) {
                return Err(
                    "artifactory_url must point to the repository under /ui/repos/tree/General/."
                        .into(),
                );
            }
            let mut api = url.clone();
            api.set_path(&url.path().replacen(marker, "/api/storage/", 1));
            result.artifact_folder_url_template = append(&api, "/{repo}");
        }
        Ok(result)
    }

    pub fn templates(&self) -> [&str; 5] {
        [
            &self.artifact_folder_url_template,
            &self.artifact_page_url_template,
            &self.build_logs_url_template,
            &self.slaps_url_template,
            &self.repository_url_template,
        ]
    }

    pub fn validate(&self) -> Result<(), String> {
        let resolved = self.resolved()?;
        if !["", "security_token", "api_key"].contains(&resolved.scm_auth.as_str()) {
            return Err("Unsupported SCM authentication mode.".into());
        }
        for template in resolved
            .templates()
            .into_iter()
            .filter(|value| !value.is_empty())
        {
            let value = expand(template, &resolved, "sample-repo", "sample-id", "1.2.3")?;
            let parsed = url::Url::parse(&value)
                .map_err(|_| "Enter absolute HTTP(S) URL templates.".to_owned())?;
            if !["http", "https"].contains(&parsed.scheme())
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(
                    "URL templates must use HTTP(S), without credentials or fragments.".into(),
                );
            }
        }
        if !resolved.artifact_folder_url_template.is_empty()
            && !(resolved.artifact_folder_url_template.contains("{repo}")
                || resolved.artifact_folder_url_template.contains("{image}"))
        {
            return Err("The artifact folder URL must contain {repo} or {image}.".into());
        }
        if !resolved.artifact_page_url_template.is_empty()
            && !resolved.artifact_page_url_template.contains("{version}")
        {
            return Err("The artifact page URL must contain {version}.".into());
        }
        Ok(())
    }

    pub fn permits(&self, target: &url::Url) -> bool {
        if !["http", "https"].contains(&target.scheme())
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return false;
        }
        let Ok(resolved) = self.resolved() else {
            return false;
        };
        resolved
            .templates()
            .into_iter()
            .filter(|value| !value.is_empty())
            .any(|template| {
                expand(template, &resolved, "sample-repo", "sample-id", "1.2.3")
                    .ok()
                    .and_then(|value| url::Url::parse(&value).ok())
                    .is_some_and(|url| url.origin() == target.origin())
            })
    }

    pub fn artifacts_configured(&self) -> bool {
        !self.artifact_folder_url_template.is_empty()
            && !self.artifact_page_url_template.is_empty()
            && [
                ("team", &self.team),
                ("region", &self.region),
                ("context", &self.context),
                ("artifactRepository", &self.artifact_repository),
            ]
            .iter()
            .all(|(key, value)| {
                !value.is_empty()
                    || ![
                        &self.artifact_folder_url_template,
                        &self.artifact_page_url_template,
                    ]
                    .iter()
                    .any(|template| template.contains(&format!("{{{key}}}")))
            })
    }
}

pub fn load(db: &Connection) -> rusqlite::Result<ImageHubConfig> {
    let value: Option<String> = db
        .query_row(
            "SELECT config_json FROM image_hub_settings WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|_| {
            rusqlite::Error::InvalidParameterName("Invalid Image Hub configuration.".into())
        }),
        None => Ok(ImageHubConfig::default()),
    }
}

pub fn save(db: &Connection, value: &ImageHubConfig) -> rusqlite::Result<()> {
    value
        .validate()
        .map_err(rusqlite::Error::InvalidParameterName)?;
    let json = serde_json::to_string(value).map_err(|_| {
        rusqlite::Error::InvalidParameterName("Cannot serialize Image Hub configuration.".into())
    })?;
    let transaction = db.unchecked_transaction()?;
    transaction.execute("INSERT INTO image_hub_settings(id, config_json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET config_json=excluded.config_json, updated_at=CURRENT_TIMESTAMP", [json])?;
    transaction.execute("DELETE FROM image_hub_latest_images", [])?;
    transaction.commit()
}

pub fn save_entry(
    db: &Connection,
    original_name: Option<&str>,
    name: &str,
    value: &str,
) -> rusqlite::Result<ImageHubConfig> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(rusqlite::Error::InvalidParameterName(
            "Enter a setting name of 1–120 characters on one line.".into(),
        ));
    }
    if value.trim().is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Value cannot be empty.".into(),
        ));
    }
    let current = load(db)?;
    let mut values = serde_json::to_value(&current).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let values_map = values
        .as_object_mut()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    if original_name != Some(name)
        && values_map
            .get(name)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "A setting with this name already exists.".into(),
        ));
    }
    if let Some(original) = original_name {
        if !values_map
            .get(original)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "This setting no longer exists. Reload Config and try again.".into(),
            ));
        }
        values_map.remove(original);
    }
    values_map.insert(name.into(), serde_json::Value::String(value.trim().into()));
    let updated: ImageHubConfig = serde_json::from_value(values)
        .map_err(|_| rusqlite::Error::InvalidParameterName("Invalid setting value.".into()))?;
    save(db, &updated)?;
    Ok(updated)
}

pub fn delete_entry(db: &Connection, name: &str) -> rusqlite::Result<ImageHubConfig> {
    let current = load(db)?;
    let mut values = serde_json::to_value(&current).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let values_map = values
        .as_object_mut()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    if values_map.remove(name).is_none() {
        return Err(rusqlite::Error::InvalidParameterName(
            "This setting no longer exists.".into(),
        ));
    }
    let updated: ImageHubConfig =
        serde_json::from_value(values).map_err(|_| rusqlite::Error::InvalidQuery)?;
    save(db, &updated)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scm_name_value_settings_resolve_and_validate_the_authentication_mode() {
        let mut config: ImageHubConfig = serde_json::from_value(
            serde_json::json!({"scm_profile": "TEST_SESSION", "scm_auth": "security_token"}),
        )
        .unwrap();
        config.validate().unwrap();
        let resolved = config.resolved().unwrap();
        assert_eq!(resolved.scm_profile, "TEST_SESSION");
        assert_eq!(resolved.scm_auth, "security_token");
        assert!(config.scm_profile.is_empty());
        config.extra.insert("scm_auth".into(), "unsupported".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn named_base_urls_resolve_without_changing_saved_entries() {
        let config: ImageHubConfig = serde_json::from_value(serde_json::json!({
            "artifactory_url": "https://artifacts.example.test/ui/repos/tree/General/releases/",
            "build_log_url": "https://build.example.test/team/projects/",
            "security_scan_url": "https://security.example.test/artifact?scope=all&artifact=old&limit=1"
        })).unwrap();
        config.validate().unwrap();
        let resolved = config.resolved().unwrap();
        assert_eq!(
            resolved.artifact_folder_url_template,
            "https://artifacts.example.test/api/storage/releases/{repo}"
        );
        assert_eq!(
            resolved.artifact_page_url_template,
            "https://artifacts.example.test/ui/repos/tree/General/releases/{repo}/{version}"
        );
        assert_eq!(
            resolved.build_logs_url_template,
            "https://build.example.test/team/projects/{repo}"
        );
        let scan = url::Url::parse(
            &expand(
                &resolved.slaps_url_template,
                &resolved,
                "service & api",
                "",
                "",
            )
            .unwrap(),
        )
        .unwrap();
        let query = scan
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.get("artifact").unwrap(), "service & api");
        assert_eq!(query.get("limit").unwrap(), "15");
        assert_eq!(query.get("scope").unwrap(), "all");
        assert!(config.permits(&scan));
        assert!(resolved.artifacts_configured());
        assert!(config.templates().iter().all(|value| value.is_empty()));
        assert_eq!(config.extra.len(), 3);
        assert_eq!(resolved.resolved().unwrap(), resolved);

        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        save(&db, &config).unwrap();
        assert_eq!(load(&db).unwrap(), config);
        let snapshot = super::super::service::snapshot(&db).unwrap();
        assert_eq!(snapshot.config, config);
        assert_eq!(snapshot.runtime_config, resolved);
    }

    #[test]
    fn name_value_entries_can_be_added_renamed_and_deleted_without_losing_other_settings() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        save_entry(&db, None, "team", "sample-team").unwrap();
        save_entry(&db, None, "Friendly name", "Production").unwrap();
        let renamed =
            save_entry(&db, Some("Friendly name"), "Environment name", "Staging").unwrap();
        assert_eq!(renamed.team, "sample-team");
        assert_eq!(
            renamed.extra.get("Environment name").map(String::as_str),
            Some("Staging")
        );
        assert!(!renamed.extra.contains_key("Friendly name"));
        assert!(save_entry(&db, Some("Environment name"), "team", "other-team").is_err());
        assert_eq!(load(&db).unwrap(), renamed);
        assert!(save_entry(&db, None, "buildLogsUrlTemplate", "javascript:alert(1)").is_err());
        assert!(save_entry(&db, None, "", "value").is_err());
        assert!(save_entry(&db, None, "entry", " ").is_err());
        let deleted = delete_entry(&db, "team").unwrap();
        assert!(deleted.team.is_empty());
        assert_eq!(
            deleted.extra.get("Environment name").map(String::as_str),
            Some("Staging")
        );
    }

    #[test]
    fn workspace_configuration_round_trips_and_invalidates_artifact_cache() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute(
            "INSERT INTO image_hub_repositories(id,name,repo_type) VALUES('repo','sample','scm')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO image_hub_latest_images(repository_id,image_name,status,checked_at) VALUES('repo','sample','not_found',1)", []).unwrap();
        let settings = ImageHubConfig {
            build_logs_url_template: "https://build.example.test/teams/{team}/projects/{repo}"
                .into(),
            team: "sample-team".into(),
            ..ImageHubConfig::default()
        };
        save(&db, &settings).unwrap();
        assert_eq!(load(&db).unwrap(), settings);
        assert!(super::super::artifacts::load(&db).unwrap().is_empty());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM image_hub_repositories", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn endpoints_are_configuration_only_and_placeholder_values_are_encoded() {
        let empty = ImageHubConfig::default();
        assert!(empty.templates().iter().all(|value| value.is_empty()));
        let config = ImageHubConfig {
            repository_url_template:
                "https://source.example.test/repositories/{ocid}?region={region}&context={context}"
                    .into(),
            region: "region-a".into(),
            context: "a,b & c".into(),
            ..empty
        };
        config.validate().unwrap();
        let url = expand(
            &config.repository_url_template,
            &config,
            "repo",
            "id/with space",
            "",
        )
        .unwrap();
        assert!(url.contains("id%2Fwith%20space"));
        assert!(url.contains("a%2Cb%20%26%20c"));
        assert!(config.permits(&url::Url::parse(&url).unwrap()));
        assert!(!config.permits(&url::Url::parse("https://different.example.test/").unwrap()));
        assert!(expand("https://example.test/{unknown}", &config, "", "", "").is_err());
        assert!(
            ImageHubConfig {
                repository_url_template: "javascript:alert(1)".into(),
                ..config
            }
            .validate()
            .is_err()
        );
    }
}
