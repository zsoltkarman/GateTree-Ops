use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnsibleImage {
    pub image_name: String,
    pub environment: String,
    pub version: Option<String>,
    pub role: String,
    pub source: String,
    pub error: Option<String>,
}

// Only scalar top-level variables affect image selection. Ignore service lists and
// nested settings, including duplicate nested keys accepted by Ansible.
struct ScalarVariable(Option<Value>);
impl<'de> serde::Deserialize<'de> for ScalarVariable {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ScalarVariable;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an Ansible variable")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ScalarVariable(Some(Value::String(value.into()))))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(ScalarVariable(Some(Value::String(value))))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ScalarVariable(Some(Value::String(value.to_string()))))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(ScalarVariable(Some(Value::String(value.to_string()))))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(ScalarVariable(Some(Value::String(value.to_string()))))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(ScalarVariable(None))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ScalarVariable(None))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(ScalarVariable(None))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(ScalarVariable(None))
            }
            fn visit_enum<A: serde::de::EnumAccess<'de>>(
                self,
                data: A,
            ) -> Result<Self::Value, A::Error> {
                use serde::de::VariantAccess;
                data.variant::<serde::de::IgnoredAny>()?
                    .1
                    .newtype_variant::<serde::de::IgnoredAny>()?;
                Ok(ScalarVariable(None))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

fn read_vars(root: &Path, path: &Path) -> Result<Mapping, String> {
    if !path.exists() {
        return Ok(Mapping::new());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| format!("Cannot read {}", path.display()))?;
    if !canonical.starts_with(root) {
        return Err("Ansible variable files must stay inside ansible_dir.".into());
    }
    if canonical.metadata().map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
        return Err(format!("Variable file is too large: {}", path.display()));
    }
    let text = std::fs::read_to_string(&canonical)
        .map_err(|_| format!("Cannot read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Mapping::new());
    }
    let values: Option<BTreeMap<String, ScalarVariable>> =
        serde_yaml::from_str(&text).map_err(|error| {
            format!(
                "Cannot parse YAML: {} at {:?}",
                path.display(),
                error.location()
            )
        })?;
    Ok(values
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (Value::String(key), value.0.unwrap_or(Value::Null)))
        .collect())
}

fn scalar(values: &Mapping, key: &str, depth: usize) -> Option<String> {
    if depth > 10 {
        return None;
    }
    match values.get(Value::String(key.into()))? {
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => {
            let value = value.trim();
            if let Some(alias) = value.strip_prefix("{{").and_then(|v| v.strip_suffix("}}")) {
                return scalar(values, alias.trim(), depth + 1);
            }
            if value.is_empty() || value.contains("{{") || value.contains("{%") {
                None
            } else {
                Some(value.into())
            }
        }
        _ => None,
    }
}

fn files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = std::fs::read_dir(path)
        .map_err(|_| format!("Cannot read {}", path.display()))?
        .map(|item| item.map(|entry| entry.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}

/// Reads the project's explicit defaults -> global -> environment include order.
/// No playbooks, lookups, plugins, or arbitrary Jinja expressions are executed.
pub fn collect(directory: &str) -> Result<Vec<AnsibleImage>, String> {
    if directory.trim().is_empty() {
        return Err("Set ansible_dir in Image Hub → Config.".into());
    }
    let directory = Path::new(directory);
    if !directory.is_absolute() {
        return Err("ansible_dir must be an absolute local path.".into());
    }
    let root = directory
        .canonicalize()
        .map_err(|_| "Cannot open ansible_dir.".to_owned())?;
    let roles = files(&root.join("roles"))?;
    let mut data = Vec::new();
    let mut environments = BTreeSet::new();
    for path in roles.into_iter().filter(|path| path.is_dir()) {
        if !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&root)
        {
            return Err("Roles must stay inside ansible_dir.".into());
        }
        let Some(role) = path.file_name().and_then(|v| v.to_str()).map(str::to_owned) else {
            continue;
        };
        let mut base = Mapping::new();
        let mut base_error = None;
        for name in [
            "defaults/main.yml".to_string(),
            "defaults/main.yaml".into(),
            format!("vars/{role}_global.yml"),
            format!("vars/{role}_global.yaml"),
        ] {
            match read_vars(&root, &path.join(name)) {
                Ok(values) => base.extend(values),
                Err(error) => base_error = Some(error),
            }
        }
        let mut overrides = BTreeMap::new();
        if path.join("vars").is_dir() {
            for file in files(&path.join("vars"))? {
                if !matches!(
                    file.extension().and_then(|v| v.to_str()),
                    Some("yml" | "yaml")
                ) {
                    continue;
                }
                let Some(env) = file
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .and_then(|v| v.strip_prefix(&format!("{role}_")))
                else {
                    continue;
                };
                if env == "global" {
                    continue;
                }
                let env = env.to_owned();
                environments.insert(env.clone());
                if overrides.insert(env.clone(), file).is_some() {
                    return Err(format!(
                        "Duplicate environment variable files for {role}/{env}"
                    ));
                }
            }
        }
        data.push((role, path, base, overrides, base_error));
    }
    let mut images = Vec::new();
    for (role, path, base, overrides, base_error) in data {
        for env in &environments {
            let mut values = base.clone();
            let file = overrides.get(env);
            let mut file_error = base_error.clone();
            if let Some(file) = file {
                match read_vars(&root, file) {
                    Ok(overrides) => values.extend(overrides),
                    Err(error) => file_error = Some(error),
                }
            }
            for key in values
                .keys()
                .filter_map(Value::as_str)
                .filter(|k| k.ends_with("IMAGE_NAME"))
            {
                let Some(name) = scalar(&values, key, 0) else {
                    continue;
                };
                let version_key =
                    key.strip_suffix("IMAGE_NAME").unwrap().to_owned() + "IMAGE_VERSION";
                let version = if file_error.is_none() {
                    scalar(&values, &version_key, 0)
                } else {
                    None
                };
                let error = file_error.clone().or_else(|| {
                    if version.is_none() {
                        Some("Image version is missing or uses an unsupported expression.".into())
                    } else {
                        None
                    }
                });
                images.push(AnsibleImage {
                    image_name: name,
                    environment: env.clone(),
                    version,
                    role: role.clone(),
                    source: file
                        .unwrap_or(&path)
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                    error,
                });
            }
        }
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires an explicit local Ansible directory"]
    fn configured_ansible_directory_live() {
        let directory = std::env::var("GATETREE_IMAGE_HUB_TEST_ANSIBLE_DIR").unwrap();
        let images = collect(&directory).unwrap();
        assert!(!images.is_empty());
        let environments = images
            .iter()
            .map(|image| image.environment.as_str())
            .collect::<BTreeSet<_>>();
        println!(
            "{} image/environment rows; environments: {:?}; {} resolved versions",
            images.len(),
            environments,
            images
                .iter()
                .filter(|image| image.version.is_some())
                .count()
        );
    }

    #[test]
    fn environment_overrides_global_and_defaults_and_preserves_image_names() {
        let root = std::env::temp_dir().join(format!("gatetree-ansible-{}", uuid::Uuid::new_v4()));
        let role = root.join("roles/sample");
        std::fs::create_dir_all(role.join("defaults")).unwrap();
        std::fs::create_dir_all(role.join("vars")).unwrap();
        std::fs::write(
            role.join("defaults/main.yml"),
            "IMAGE_NAME: sample-image\nIMAGE_VERSION: 1.0.0\n",
        )
        .unwrap();
        std::fs::write(
            role.join("vars/sample_global.yml"),
            "IMAGE_VERSION: 2.0.0\nSERVICES:\n  - name: one\n    setting: first\n    setting: last\n",
        )
        .unwrap();
        std::fs::write(
            role.join("vars/sample_test.yml"),
            "IMAGE_VERSION: '{{ RELEASE }}'\nRELEASE: 3.0.0\n",
        )
        .unwrap();
        std::fs::write(role.join("vars/sample_stage.yml"), "{}\n").unwrap();
        let images = collect(root.to_str().unwrap()).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(
            images
                .iter()
                .find(|i| i.environment == "test")
                .unwrap()
                .version
                .as_deref(),
            Some("3.0.0")
        );
        assert_eq!(
            images
                .iter()
                .find(|i| i.environment == "stage")
                .unwrap()
                .version
                .as_deref(),
            Some("2.0.0")
        );
        assert!(images.iter().all(|i| i.image_name == "sample-image"));
        // An environment used by another role still inherits this role's global version.
        let other = root.join("roles/other/vars");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("other_stage.yml"), "{}\n").unwrap();
        std::fs::remove_file(role.join("vars/sample_stage.yml")).unwrap();
        assert_eq!(
            collect(root.to_str().unwrap())
                .unwrap()
                .iter()
                .find(|i| i.environment == "stage")
                .unwrap()
                .version
                .as_deref(),
            Some("2.0.0")
        );
        std::fs::write(
            role.join("vars/sample_test.yml"),
            "IMAGE_VERSION: '{{ missing | plugin }}'\n",
        )
        .unwrap();
        assert!(
            collect(root.to_str().unwrap())
                .unwrap()
                .iter()
                .find(|i| i.environment == "test")
                .unwrap()
                .error
                .is_some()
        );
        std::fs::write(role.join("vars/sample_test.yml"), "IMAGE_VERSION: [bad\n").unwrap();
        let images = collect(root.to_str().unwrap()).unwrap();
        assert!(
            images
                .iter()
                .find(|i| i.environment == "test")
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("Cannot parse YAML")
        );
        assert_eq!(
            images
                .iter()
                .find(|i| i.environment == "stage")
                .unwrap()
                .version
                .as_deref(),
            Some("2.0.0")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
