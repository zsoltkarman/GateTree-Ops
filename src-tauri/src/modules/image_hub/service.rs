use rusqlite::{Connection, params};
use uuid::Uuid;

use super::model::{
    FolderInput, ImageHubFolder, ImageHubSnapshot, Project, ProjectInput, Repository,
    RepositoryInput,
};

fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_owned())
}

fn required(value: &str) -> rusqlite::Result<&str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid("Name cannot be empty."));
    }
    Ok(value)
}

pub(super) fn is_ssh_shorthand(value: &str) -> bool {
    !value.contains("://")
        && value.split_once('@').is_some_and(|(user, rest)| {
            !user.is_empty()
                && rest.split_once(':').is_some_and(|(host, path)| {
                    !host.is_empty() && !path.is_empty() && !value.chars().any(char::is_whitespace)
                })
        })
}

pub fn project(connection: &Connection, id: &str) -> rusqlite::Result<Project> {
    connection.query_row(
        "SELECT id, folder_id, name, local_path, project_type, source FROM image_hub_projects WHERE id = ?1",
        [id], |r| Ok(Project { id: r.get(0)?, folder_id: r.get(1)?, name: r.get(2)?,
            local_path: r.get(3)?, project_type: r.get(4)?, source: r.get(5)? }),
    )
}

pub fn save_project(connection: &Connection, input: ProjectInput) -> rusqlite::Result<String> {
    let name = required(&input.name)?;
    check_folder(connection, input.folder_id.as_deref())?;
    if !["scm", "gitlab", "cloudlab"].contains(&input.project_type.as_str()) {
        return Err(invalid("Project type must be SCM, GitLab or CloudLab."));
    }
    if !std::path::Path::new(input.local_path.trim()).is_absolute() {
        return Err(invalid("Choose an absolute local path for the project."));
    }
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let values = params![
        id,
        input.folder_id,
        name,
        input.local_path.trim(),
        input.project_type,
        input.source.trim()
    ];
    if input.id.is_some() {
        let previous = project(connection, &id)?;
        let has_repos: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_hub_repositories WHERE project_id=?1)",
            [&id],
            |r| r.get(0),
        )?;
        if has_repos
            && (previous.project_type != input.project_type
                || (!previous.source.is_empty() && previous.source != input.source.trim())
                || previous.local_path != input.local_path.trim())
        {
            return Err(invalid(
                "This project already contains repositories. Create a separate project to change its source, type or local path.",
            ));
        }
        connection.execute("UPDATE image_hub_projects SET folder_id=?2, name=?3, local_path=?4, project_type=?5, source=?6, updated_at=CURRENT_TIMESTAMP WHERE id=?1", values)?;
        connection.execute(
            "UPDATE image_hub_repositories SET folder_id=?2 WHERE project_id=?1",
            params![id, input.folder_id],
        )?;
    } else {
        connection.execute("INSERT INTO image_hub_projects(id, folder_id, name, local_path, project_type, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", values)?;
    }
    Ok(id)
}

pub fn delete_project(connection: &Connection, id: &str) -> rusqlite::Result<()> {
    let count: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM image_hub_repositories WHERE project_id=?1)",
        [id],
        |r| r.get(0),
    )?;
    if count {
        return Err(invalid(
            "Remove or move the repositories before deleting this project. Local files are kept.",
        ));
    }
    if connection.execute("DELETE FROM image_hub_projects WHERE id=?1", [id])? == 0 {
        return Err(invalid("The project no longer exists."));
    }
    Ok(())
}

fn check_folder(connection: &Connection, id: Option<&str>) -> rusqlite::Result<()> {
    if let Some(id) = id {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_hub_folders WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(invalid("The selected folder no longer exists."));
        }
    }
    Ok(())
}

pub fn snapshot(connection: &Connection) -> rusqlite::Result<ImageHubSnapshot> {
    let mut folders = connection.prepare(
        "SELECT id, name, parent_id FROM image_hub_folders ORDER BY name COLLATE NOCASE, id",
    )?;
    let folders = folders
        .query_map([], |r| {
            Ok(ImageHubFolder {
                id: r.get(0)?,
                name: r.get(1)?,
                parent_id: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut repositories = connection.prepare(
        "SELECT id, folder_id, name, local_path, ocid, url, repo_type, project_id
         FROM image_hub_repositories ORDER BY name COLLATE NOCASE, id",
    )?;
    let mut repositories = repositories
        .query_map([], |r| {
            Ok(Repository {
                id: r.get(0)?,
                folder_id: r.get(1)?,
                name: r.get(2)?,
                local_path: r.get(3)?,
                ocid: r.get(4)?,
                url: r.get(5)?,
                repo_type: r.get(6)?,
                browser_url: None,
                project_id: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut projects = connection.prepare(
        "SELECT id, folder_id, name, local_path, project_type, source FROM image_hub_projects ORDER BY name COLLATE NOCASE, id"
    )?;
    let projects = projects
        .query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                folder_id: r.get(1)?,
                name: r.get(2)?,
                local_path: r.get(3)?,
                project_type: r.get(4)?,
                source: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for repo in &mut repositories {
        if repo.repo_type == "scm" {
            if let Some(project) = projects.iter().find(|project| {
                Some(&project.id) == repo.project_id.as_ref() && project.project_type == "scm"
            }) {
                repo.browser_url =
                    super::project_sync::scm_repository_browser_url(&project.source, &repo.ocid);
            }
        }
    }
    let config = super::config::load(connection)?;
    let runtime_config = config
        .resolved()
        .map_err(rusqlite::Error::InvalidParameterName)?;
    Ok(ImageHubSnapshot {
        folders,
        repositories,
        projects,
        latest_images: super::artifacts::load(connection)?,
        security_reports: super::security::load_summaries(connection, &runtime_config)?,
        sync_statuses: super::sync_job::load(connection)?,
        config,
        runtime_config,
    })
}

pub fn save_folder(connection: &Connection, input: FolderInput) -> rusqlite::Result<String> {
    let name = required(&input.name)?;
    check_folder(connection, input.parent_id.as_deref())?;
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if input.id.is_some() {
        check_folder(connection, Some(&id))?;
        let cycle: bool = connection.query_row(
            "WITH RECURSIVE descendants(id) AS (
                SELECT id FROM image_hub_folders WHERE id = ?1
                UNION SELECT f.id FROM image_hub_folders f JOIN descendants d ON f.parent_id = d.id
             ) SELECT EXISTS(SELECT 1 FROM descendants WHERE id = ?2)",
            params![id, input.parent_id],
            |r| r.get(0),
        )?;
        if cycle {
            return Err(invalid(
                "A folder cannot be moved into itself or its subfolders.",
            ));
        }
        connection.execute(
            "UPDATE image_hub_folders SET name = ?2, parent_id = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![id, name, input.parent_id],
        )?;
    } else {
        connection.execute(
            "INSERT INTO image_hub_folders(id, name, parent_id) VALUES (?1, ?2, ?3)",
            params![id, name, input.parent_id],
        )?;
    }
    Ok(id)
}

pub fn save_repository(
    connection: &Connection,
    mut input: RepositoryInput,
) -> rusqlite::Result<String> {
    let name = required(&input.name)?;
    check_folder(connection, input.folder_id.as_deref())?;
    if let Some(project_id) = input.project_id.as_deref() {
        input.folder_id = project(connection, project_id)?.folder_id;
    }
    if !["cloudlab", "scm", "github", "gitlab"].contains(&input.repo_type.as_str()) {
        return Err(invalid(
            "Repository type must be CloudLab, SCM, GitLab or GitHub.",
        ));
    }
    let repo_url = input.url.trim();
    if !repo_url.is_empty() && !is_ssh_shorthand(repo_url) {
        let url = url::Url::parse(repo_url)
            .map_err(|_| invalid("Enter a valid repository URL (https:// or ssh://)."))?;
        if !["http", "https", "ssh", "git"].contains(&url.scheme()) || url.host_str().is_none() {
            return Err(invalid(
                "Enter a repository URL using HTTP, HTTPS, SSH or Git.",
            ));
        }
    }
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let values = params![
        id,
        input.folder_id,
        name,
        input.local_path.trim(),
        input.ocid.trim(),
        repo_url,
        input.repo_type,
        input.project_id
    ];
    if input.id.is_some() {
        let changed = connection.execute(
            "UPDATE image_hub_repositories SET folder_id = ?2, name = ?3, local_path = ?4,
                ocid = ?5, url = ?6, repo_type = ?7, project_id = ?8, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            values,
        )?;
        if changed == 0 {
            return Err(invalid("The repository no longer exists."));
        }
    } else {
        connection.execute(
            "INSERT INTO image_hub_repositories(id, folder_id, name, local_path, ocid, url, repo_type, project_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", values,
        )?;
    }
    Ok(id)
}

pub fn delete_folder(connection: &Connection, id: &str) -> rusqlite::Result<()> {
    check_folder(connection, Some(id))?;
    let occupied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM image_hub_folders WHERE parent_id = ?1)
             OR EXISTS(SELECT 1 FROM image_hub_repositories WHERE folder_id = ?1)
             OR EXISTS(SELECT 1 FROM image_hub_projects WHERE folder_id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    if occupied {
        return Err(invalid(
            "Move or delete the subfolders, projects and repositories before deleting this folder.",
        ));
    }
    connection.execute("DELETE FROM image_hub_folders WHERE id = ?1", [id])?;
    Ok(())
}

pub fn delete_repository(connection: &Connection, id: &str) -> rusqlite::Result<()> {
    if connection.execute("DELETE FROM image_hub_repositories WHERE id = ?1", [id])? == 0 {
        return Err(invalid("The repository no longer exists."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_keep_repository_links_and_allow_configuring_source_after_local_import() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        let parent = save_folder(
            &db,
            FolderInput {
                id: None,
                name: "Apps".into(),
                parent_id: None,
            },
        )
        .unwrap();
        let mut input = ProjectInput {
            id: None,
            folder_id: Some(parent.clone()),
            name: "Sample".into(),
            local_path: std::env::temp_dir()
                .join("sample")
                .to_string_lossy()
                .into_owned(),
            project_type: "gitlab".into(),
            source: String::new(),
        };
        let id = save_project(&db, input.clone()).unwrap();
        input.id = Some(id.clone());
        let repo = save_repository(
            &db,
            RepositoryInput {
                id: None,
                folder_id: Some(parent.clone()),
                project_id: Some(id.clone()),
                name: "one".into(),
                local_path: "/repo".into(),
                ocid: String::new(),
                url: "git@git.example.com:team/one.git".into(),
                repo_type: "gitlab".into(),
            },
        )
        .unwrap();
        assert!(delete_folder(&db, &parent).is_err());
        assert!(delete_project(&db, &id).is_err());
        input.source = "https://git.example.com/team".into();
        save_project(&db, input.clone()).unwrap();
        input.folder_id = None;
        save_project(&db, input.clone()).unwrap();
        assert!(snapshot(&db).unwrap().repositories[0].folder_id.is_none());
        assert_eq!(
            snapshot(&db).unwrap().repositories[0].project_id.as_deref(),
            Some(id.as_str())
        );
        delete_folder(&db, &parent).unwrap();
        input.local_path.push_str("-changed");
        assert!(save_project(&db, input).is_err());
        delete_repository(&db, &repo).unwrap();
        delete_project(&db, &id).unwrap();
    }

    #[test]
    fn inventory_persists_when_the_encrypted_workspace_is_reopened() {
        use crate::storage::EncryptedDatabase;
        let directory = std::env::temp_dir().join(format!("image-hub-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("inventory.sqlite");
        let key = [42; 32];
        let database = EncryptedDatabase::create_with_key(&path, &key).unwrap();
        let folder = save_folder(
            database.connection(),
            FolderInput {
                id: None,
                name: "Sample".into(),
                parent_id: None,
            },
        )
        .unwrap();
        save_repository(
            database.connection(),
            RepositoryInput {
                project_id: None,
                id: None,
                name: "sample-service".into(),
                folder_id: Some(folder),
                local_path: "/work/sample".into(),
                ocid: "".into(),
                url: "https://git.example.com/sample".into(),
                repo_type: "cloudlab".into(),
            },
        )
        .unwrap();
        drop(database);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"));
        let database = EncryptedDatabase::open_with_key(&path, &key).unwrap();
        let result = snapshot(database.connection()).unwrap();
        assert_eq!(result.folders[0].name, "Sample");
        assert_eq!(result.repositories[0].name, "sample-service");
        assert_eq!(result.repositories[0].repo_type, "cloudlab");
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn nested_inventory_survives_reload_and_rejects_cycles_and_nonempty_deletion() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        let folder = |id, name: &str, parent_id| FolderInput {
            id,
            name: name.into(),
            parent_id,
        };
        let parent = save_folder(&db, folder(None, "Sample", None)).unwrap();
        let child = save_folder(&db, folder(None, "Collectors", Some(parent.clone()))).unwrap();
        assert!(
            save_folder(
                &db,
                folder(Some(parent.clone()), "Sample", Some(child.clone()))
            )
            .is_err()
        );
        assert!(
            save_folder(
                &db,
                folder(Some(child.clone()), "Collectors", Some(child.clone()))
            )
            .is_err()
        );
        assert!(save_folder(&db, folder(None, " ", None)).is_err());
        let repo = |id, folder_id, repo_type: &str| RepositoryInput {
            project_id: None,
            id,
            folder_id,
            name: "collector".into(),
            local_path: "/tmp/repo with spaces".into(),
            ocid: "ocid1.devopsrepository.test".into(),
            url: "ssh://git.example.com/repo".into(),
            repo_type: repo_type.into(),
        };
        assert!(save_repository(&db, repo(None, Some("missing".into()), "scm")).is_err());
        assert!(save_repository(&db, repo(None, None, "other")).is_err());
        let id = save_repository(&db, repo(None, Some(child.clone()), "scm")).unwrap();
        assert!(delete_folder(&db, &parent).is_err());
        assert!(delete_folder(&db, &child).is_err());
        let loaded = snapshot(&db).unwrap();
        assert_eq!(loaded.repositories[0].local_path, "/tmp/repo with spaces");
        assert_eq!(loaded.repositories[0].ocid, "ocid1.devopsrepository.test");
        assert_eq!(
            loaded.repositories[0].folder_id.as_deref(),
            Some(child.as_str())
        );
        assert_eq!(
            loaded
                .folders
                .iter()
                .find(|f| f.id == child)
                .unwrap()
                .parent_id
                .as_deref(),
            Some(parent.as_str())
        );
        save_repository(&db, repo(Some(id.clone()), None, "github")).unwrap();
        save_folder(&db, folder(Some(child.clone()), "Renamed", None)).unwrap();
        delete_folder(&db, &parent).unwrap();
        delete_folder(&db, &child).unwrap();
        assert_eq!(snapshot(&db).unwrap().repositories[0].repo_type, "github");
        delete_repository(&db, &id).unwrap();
        assert!(snapshot(&db).unwrap().repositories.is_empty());
        crate::storage::migrations::run_migrations(&mut db).unwrap();
    }
}

/// A manually saved HTTP(S) repository URL is also an explicit browser destination.
pub fn permits_repository_url(
    db: &rusqlite::Connection,
    target: &url::Url,
) -> rusqlite::Result<bool> {
    if !["http", "https"].contains(&target.scheme())
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Ok(false);
    }
    let mut statement = db.prepare("SELECT r.url,r.ocid,CASE WHEN r.repo_type='scm' AND p.project_type='scm' THEN p.source ELSE '' END FROM image_hub_repositories r LEFT JOIN image_hub_projects p ON p.id=r.project_id")?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (saved_url, ocid, source) = row?;
        if url::Url::parse(&saved_url).ok().as_ref() == Some(target)
            || super::project_sync::scm_repository_browser_url(&source, &ocid)
                .and_then(|url| url::Url::parse(&url).ok())
                .as_ref()
                == Some(target)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod repository_url_tests {
    use super::*;
    #[test]
    fn synced_repository_ocid_and_saved_project_url_resolve_the_browser_link_without_config() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        let source = "https://console.example.test/devops-project/projects/ocid1.devopsproject.oc1.test.project/overview?_ctx=region-a%2Caccount-a";
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Sample','/sample','scm',?1)", [source]).unwrap();
        db.execute("INSERT INTO image_hub_repositories(id,name,repo_type,project_id,ocid,url) VALUES('repo','sample','scm','project','ocid1.devopsrepository.oc1.test.repository','ssh://git.example.test/sample')", []).unwrap();
        let snapshot = snapshot(&db).unwrap();
        let repo = &snapshot.repositories[0];
        let url = url::Url::parse(repo.browser_url.as_deref().unwrap()).unwrap();
        assert_eq!(repo.url, "ssh://git.example.test/sample");
        assert!(snapshot.config.repository_url_template.is_empty());
        assert_eq!(url.host_str(), Some("console.example.test"));
        assert!(permits_repository_url(&db, &url).unwrap());
        db.execute(
            "UPDATE image_hub_projects SET source='ocid1.devopsproject.oc1.test.project'",
            [],
        )
        .unwrap();
        assert!(!permits_repository_url(&db, &url).unwrap());
    }
    #[test]
    fn saved_browser_urls_are_allowed_without_allowing_other_paths_or_clone_credentials() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_repositories(id,name,repo_type,url) VALUES('repo','sample','scm','https://source.example.test/team/service')", []).unwrap();
        assert!(
            permits_repository_url(
                &db,
                &url::Url::parse("https://source.example.test/team/service").unwrap()
            )
            .unwrap()
        );
        for target in [
            "https://source.example.test/another/service",
            "https://other.example.test/team/service",
            "ssh://source.example.test/team/service",
            "https://user:password@source.example.test/team/service",
            "https://source.example.test/team/service#fragment",
        ] {
            assert!(!permits_repository_url(&db, &url::Url::parse(target).unwrap()).unwrap());
        }
    }
}
