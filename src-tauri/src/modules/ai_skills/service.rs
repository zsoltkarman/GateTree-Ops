use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};

use super::model::{
    AiSkill, AiSkillDocument, AiSkillGroup, AiSkillLibrary, AiSkillRefreshResult, AiSkillRunResult,
    AiSkillsSnapshot,
};

pub struct AiSkillsService;

impl AiSkillsService {
    pub fn snapshot(state: &WorkspaceState) -> Result<AiSkillsSnapshot, AiSkillsError> {
        state
            .with_connection(|connection| {
                let libraries = load_libraries(connection)?;
                let skills = load_skills(connection)?;
                let groups = load_catalog_groups(&libraries, &skills);
                Ok(AiSkillsSnapshot {
                    libraries,
                    skills,
                    groups,
                })
            })
            .map_err(AiSkillsError::from)
    }

    pub fn create_library(
        state: &WorkspaceState,
        name: String,
        path: String,
    ) -> Result<AiSkillLibrary, AiSkillsError> {
        let name = validate_name(name)?;
        let path = validate_library_path(path)?;
        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO ai_skill_libraries(id, name, path) VALUES (?1, ?2, ?3)",
                    params![id, name, path],
                )?;
                library_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(AiSkillsError::from)
    }

    pub fn update_library(
        state: &WorkspaceState,
        id: String,
        name: String,
        path: String,
    ) -> Result<AiSkillLibrary, AiSkillsError> {
        let name = validate_name(name)?;
        let path = validate_library_path(path)?;
        state
            .with_connection(|connection| {
                let changed = connection.execute(
                    "UPDATE ai_skill_libraries
                     SET name = ?2, path = ?3, updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![id, name, path],
                )?;
                if changed == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                connection.execute("DELETE FROM ai_skills WHERE library_id = ?1", [&id])?;
                library_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(AiSkillsError::from)
    }

    pub fn delete_library(state: &WorkspaceState, id: String) -> Result<(), AiSkillsError> {
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM ai_skill_libraries WHERE id = ?1", [id])?;
                Ok(())
            })
            .map_err(AiSkillsError::from)
    }

    pub fn refresh(state: &WorkspaceState) -> Result<AiSkillRefreshResult, AiSkillsError> {
        let libraries = state
            .with_connection(load_libraries)
            .map_err(AiSkillsError::from)?;
        if libraries.is_empty() {
            return Err(AiSkillsError::NoLibraries);
        }

        let mut discovered = Vec::new();
        let mut skipped_libraries = Vec::new();
        for library in &libraries {
            let root = PathBuf::from(&library.path);
            if !root.is_dir() {
                skipped_libraries.push(library.name.clone());
                continue;
            }
            if discover_skills(&root, &root, &library.id, &mut discovered).is_err() {
                skipped_libraries.push(library.name.clone());
            }
        }
        if discovered.is_empty() && skipped_libraries.len() == libraries.len() {
            return Err(AiSkillsError::LibrariesUnavailable);
        }

        state
            .with_connection(|connection| {
                let transaction = connection.unchecked_transaction()?;
                transaction.execute("DELETE FROM ai_skills", [])?;
                for skill in &discovered {
                    transaction.execute(
                        "INSERT INTO ai_skills(id, library_id, name, description, path)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            skill.id,
                            skill.library_id,
                            skill.name,
                            skill.description,
                            skill.path
                        ],
                    )?;
                }
                transaction.commit()
            })
            .map_err(AiSkillsError::from)?;

        Ok(AiSkillRefreshResult {
            discovered: discovered.len(),
            skipped_libraries,
        })
    }

    pub fn read_skill(
        state: &WorkspaceState,
        id: String,
    ) -> Result<AiSkillDocument, AiSkillsError> {
        let skill = state
            .with_connection(|connection| skill_by_id(connection, &id))
            .map_err(AiSkillsError::from)?
            .ok_or(AiSkillsError::SkillNotFound)?;
        let directory = fs::canonicalize(&skill.path).map_err(AiSkillsError::Io)?;
        let allowed_root = state
            .with_connection(|connection| library_by_id(connection, &skill.library_id))
            .map_err(AiSkillsError::from)?
            .ok_or(AiSkillsError::SkillNotFound)?;
        let allowed_root = fs::canonicalize(allowed_root.path).map_err(AiSkillsError::Io)?;
        if !directory.starts_with(&allowed_root) {
            return Err(AiSkillsError::OutsideLibrary);
        }
        let content = fs::read_to_string(directory.join("SKILL.md")).map_err(AiSkillsError::Io)?;
        let input_prompt = front_matter_value(&content, "input_prompt");
        Ok(AiSkillDocument {
            id: skill.id,
            name: skill.name,
            description: skill.description,
            path: skill.path,
            content,
            input_prompt,
        })
    }

    pub fn run_skill(
        state: &WorkspaceState,
        id: String,
        input: String,
    ) -> Result<AiSkillRunResult, AiSkillsError> {
        let skill = state
            .with_connection(|connection| skill_by_id(connection, &id))
            .map_err(AiSkillsError::from)?
            .ok_or(AiSkillsError::SkillNotFound)?;
        let library = state
            .with_connection(|connection| library_by_id(connection, &skill.library_id))
            .map_err(AiSkillsError::from)?
            .ok_or(AiSkillsError::SkillNotFound)?;
        let directory = fs::canonicalize(&skill.path).map_err(AiSkillsError::Io)?;
        let library_root = fs::canonicalize(&library.path).map_err(AiSkillsError::Io)?;
        if !directory.starts_with(&library_root) {
            return Err(AiSkillsError::OutsideLibrary);
        }
        let skill_file = directory.join("SKILL.md");
        if !skill_file.is_file() {
            return Err(AiSkillsError::SkillNotFound);
        }
        let request = input.trim();
        let request = if request.is_empty() {
            "Run the skill and report the result."
        } else {
            request
        };
        let prompt = format!(
            "Use the skill at {}. Read its SKILL.md and any supporting files it references, then follow its instructions.\n\nUser request:\n{}",
            skill_file.display(),
            request
        );
        let executable = codex_executable().ok_or(AiSkillsError::CodexUnavailable)?;
        let result = Command::new(executable)
            .current_dir(&directory)
            .args([
                "exec",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--skip-git-repo-check",
            ])
            .arg(prompt)
            .output()
            .map_err(AiSkillsError::Io)?;
        let stdout = String::from_utf8_lossy(&result.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        let output = match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => format!("{stdout}\n\n{stderr}"),
            (false, true) => stdout,
            (true, false) => stderr,
            (true, true) => "Codex completed without output.".to_owned(),
        };
        if !result.status.success() {
            return Err(AiSkillsError::CodexFailed(output));
        }
        Ok(AiSkillRunResult {
            output,
            status: "Completed".to_owned(),
        })
    }
}

fn validate_name(name: String) -> Result<String, AiSkillsError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        Err(AiSkillsError::EmptyName)
    } else {
        Ok(name)
    }
}

fn validate_library_path(path: String) -> Result<String, AiSkillsError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(AiSkillsError::EmptyPath);
    }
    let canonical = fs::canonicalize(path).map_err(AiSkillsError::Io)?;
    if !canonical.is_dir() {
        return Err(AiSkillsError::NotDirectory);
    }
    Ok(canonical.to_string_lossy().into_owned())
}

fn discover_skills(
    root: &Path,
    directory: &Path,
    library_id: &str,
    skills: &mut Vec<AiSkill>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            discover_skills(root, &entry.path(), library_id, skills)?;
        } else if file_type.is_file() && file_name == "SKILL.md" {
            let content = fs::read_to_string(entry.path())?;
            let skill_directory = entry.path().parent().unwrap_or(root).to_path_buf();
            let fallback_name = skill_directory
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Skill");
            let (name, description) = skill_metadata(&content, fallback_name);
            let path = skill_directory.to_string_lossy().into_owned();
            let id = stable_skill_id(library_id, &path);
            skills.push(AiSkill {
                id,
                library_id: library_id.to_owned(),
                name,
                description,
                path,
                updated_at: String::new(),
            });
        }
    }
    Ok(())
}

fn skill_metadata(content: &str, fallback_name: &str) -> (String, String) {
    let mut name = None;
    let mut description = None;
    let mut lines = content.lines();
    if lines.next().map(str::trim) == Some("---") {
        for line in lines {
            let line = line.trim();
            if line == "---" {
                break;
            }
            if let Some(value) = line.strip_prefix("name:") {
                name = Some(unquote(value.trim()));
            } else if let Some(value) = line.strip_prefix("description:") {
                description = Some(unquote(value.trim()));
            }
        }
    }
    (
        name.filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_name.to_owned()),
        description.unwrap_or_default(),
    )
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_owned()
}

fn front_matter_value(contents: &str, key: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix(&format!("{key}:")) {
            let value = unquote(value.trim());
            return (!value.is_empty()).then_some(value);
        }
    }
    None
}

fn codex_executable() -> Option<PathBuf> {
    if let Some(path) = env::var_os("GATETREE_OPS_CODEX_PATH")
        .or_else(|| env::var_os("PHAROS_CODEX_PATH"))
        .map(PathBuf::from)
    {
        if path.is_file() {
            return Some(path);
        }
    }
    #[cfg(target_os = "macos")]
    for path in ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|path| path.join(if cfg!(windows) { "codex.exe" } else { "codex" }))
            .find(|path| path.is_file())
    })
}

fn stable_skill_id(library_id: &str, path: &str) -> String {
    format!("{library_id}:{}", path.replace(':', "%3A"))
}

#[derive(Debug)]
struct CatalogDefinition {
    id: String,
    label: String,
    icon: String,
    skill_directories: Vec<String>,
}

fn load_catalog_groups(libraries: &[AiSkillLibrary], skills: &[AiSkill]) -> Vec<AiSkillGroup> {
    let mut groups = Vec::new();
    let mut assigned = HashSet::new();
    for library in libraries {
        let library_skills = skills
            .iter()
            .filter(|skill| skill.library_id == library.id)
            .collect::<Vec<_>>();
        let catalog_path = Path::new(&library.path).join("CATALOG.md");
        let definitions = fs::read_to_string(catalog_path)
            .map(|contents| parse_catalog(&contents))
            .unwrap_or_default();
        for definition in definitions {
            let skill_ids = definition
                .skill_directories
                .iter()
                .filter_map(|directory| {
                    library_skills
                        .iter()
                        .find(|skill| {
                            Path::new(&skill.path)
                                .file_name()
                                .and_then(|value| value.to_str())
                                == Some(directory.as_str())
                        })
                        .map(|skill| skill.id.clone())
                })
                .collect::<Vec<_>>();
            if skill_ids.is_empty() {
                continue;
            }
            assigned.extend(skill_ids.iter().cloned());
            groups.push(AiSkillGroup {
                id: format!("{}#{}", library.path, definition.id),
                label: definition.label,
                icon: definition.icon,
                skill_ids,
            });
        }
    }
    let ungrouped = skills
        .iter()
        .filter(|skill| !assigned.contains(&skill.id))
        .map(|skill| skill.id.clone())
        .collect::<Vec<_>>();
    if !ungrouped.is_empty() {
        groups.push(AiSkillGroup {
            id: "ungrouped".to_owned(),
            label: "Other skills".to_owned(),
            icon: "folder".to_owned(),
            skill_ids: ungrouped,
        });
    }
    groups
}

fn parse_catalog(contents: &str) -> Vec<CatalogDefinition> {
    let lines = contents.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return Vec::new();
    }
    let Some(closing_index) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (line.trim() == "---").then_some(index))
    else {
        return Vec::new();
    };

    let mut groups = Vec::<CatalogDefinition>::new();
    let mut reading_skills = false;
    for line in &lines[1..closing_index] {
        let trimmed = line.trim();
        let indentation = line
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        if indentation == 2 && trimmed.starts_with("- id:") {
            let id = trimmed["- id:".len()..].trim().to_owned();
            groups.push(CatalogDefinition {
                label: id.clone(),
                id,
                icon: "folder".to_owned(),
                skill_directories: Vec::new(),
            });
            reading_skills = false;
        } else if let Some(group) = groups.last_mut() {
            if indentation == 4 && trimmed.starts_with("label:") {
                group.label = unquote(trimmed["label:".len()..].trim());
            } else if indentation == 4 && trimmed.starts_with("icon:") {
                group.icon = unquote(trimmed["icon:".len()..].trim());
            } else if indentation == 4 && trimmed == "skills:" {
                reading_skills = true;
            } else if reading_skills && indentation >= 6 && trimmed.starts_with("- ") {
                group.skill_directories.push(trimmed[2..].trim().to_owned());
            }
        }
    }
    groups
}

fn load_libraries(connection: &Connection) -> rusqlite::Result<Vec<AiSkillLibrary>> {
    let mut statement = connection.prepare(
        "SELECT id, name, path, created_at, updated_at
         FROM ai_skill_libraries ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([], library_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn load_skills(connection: &Connection) -> rusqlite::Result<Vec<AiSkill>> {
    let mut statement = connection.prepare(
        "SELECT id, library_id, name, description, path, updated_at
         FROM ai_skills ORDER BY name COLLATE NOCASE, path COLLATE NOCASE",
    )?;
    statement
        .query_map([], skill_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn library_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<AiSkillLibrary>> {
    connection
        .query_row(
            "SELECT id, name, path, created_at, updated_at
             FROM ai_skill_libraries WHERE id = ?1",
            [id],
            library_from_row,
        )
        .optional()
}

fn skill_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<AiSkill>> {
    connection
        .query_row(
            "SELECT id, library_id, name, description, path, updated_at
             FROM ai_skills WHERE id = ?1",
            [id],
            skill_from_row,
        )
        .optional()
}

fn library_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiSkillLibrary> {
    Ok(AiSkillLibrary {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn skill_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiSkill> {
    Ok(AiSkill {
        id: row.get(0)?,
        library_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        path: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[derive(Debug, Error)]
pub enum AiSkillsError {
    #[error("Folder name cannot be empty.")]
    EmptyName,
    #[error("Choose a local skill folder.")]
    EmptyPath,
    #[error("The selected path is not a folder.")]
    NotDirectory,
    #[error("Add a Skill Library in Config before refreshing skills.")]
    NoLibraries,
    #[error("None of the configured Skill Libraries can be read.")]
    LibrariesUnavailable,
    #[error("The selected skill no longer exists in the catalog.")]
    SkillNotFound,
    #[error("The skill path is outside its configured library.")]
    OutsideLibrary,
    #[error("Codex CLI was not found. Install Codex or set GATETREE_OPS_CODEX_PATH.")]
    CodexUnavailable,
    #[error("Codex failed:\n{0}")]
    CodexFailed(String),
    #[error("Could not read the skill folder: {0}")]
    Io(#[source] std::io::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_catalog_front_matter_in_order() {
        let groups = parse_catalog(concat!(
            "---\n",
            "groups:\n",
            "  - id: on-call\n",
            "    label: On-call\n",
            "    icon: \"🚨\"\n",
            "    skills:\n",
            "      - alert-discovery\n",
            "      - incident-triage\n",
            "  - id: control-plane\n",
            "    label: Control plane\n",
            "    icon: \"⚙️\"\n",
            "    skills:\n",
            "      - my-ai\n",
            "---\n",
        ));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].label, "On-call");
        assert_eq!(groups[0].icon, "🚨");
        assert_eq!(
            groups[0].skill_directories,
            ["alert-discovery", "incident-triage"]
        );
        assert_eq!(groups[1].skill_directories, ["my-ai"]);
    }
}
