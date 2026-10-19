use std::{collections::HashSet, io};

use rusqlite::{Connection, params};
use serde::Serialize;
use tauri::State;

use super::{model::ConnectionItem, service::connection_by_id};
use crate::modules::workspace::WorkspaceState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickLaunchItem {
    pub connection: ConnectionItem,
    pub label: Option<String>,
}

fn load(connection: &Connection) -> rusqlite::Result<Vec<QuickLaunchItem>> {
    let entries = connection
        .prepare(
            "SELECT connection_id, label FROM quick_launch_items ORDER BY position, connection_id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    entries
        .into_iter()
        .map(|(id, label)| {
            connection_by_id(connection, &id)?
                .map(|connection| QuickLaunchItem { connection, label })
                .ok_or(rusqlite::Error::QueryReturnedNoRows)
        })
        .collect()
}

fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(
        io::ErrorKind::InvalidInput,
        message,
    )))
}

pub struct QuickLauncherService;

impl QuickLauncherService {
    pub fn list(state: &WorkspaceState) -> Result<Vec<QuickLaunchItem>, String> {
        state
            .with_connection(load)
            .map_err(|error| error.to_string())
    }

    pub fn pin(state: &WorkspaceState, id: &str) -> Result<Vec<QuickLaunchItem>, String> {
        state
            .with_connection(|connection| {
                if connection_by_id(connection, id)?.is_none() {
                    return Err(invalid("Connection no longer exists."));
                }
                connection.execute(
                    "INSERT OR IGNORE INTO quick_launch_items(connection_id, position)
                VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1 FROM quick_launch_items))",
                    [id],
                )?;
                load(connection)
            })
            .map_err(|error| error.to_string())
    }

    pub fn rename(
        state: &WorkspaceState,
        id: &str,
        label: Option<String>,
    ) -> Result<Vec<QuickLaunchItem>, String> {
        let label = label
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if label
            .as_ref()
            .is_some_and(|value| value.chars().count() > 80)
        {
            return Err("Launcher labels must be at most 80 characters.".into());
        }
        state
            .with_connection(|connection| {
                if connection.execute(
                    "UPDATE quick_launch_items SET label=?2 WHERE connection_id=?1",
                    params![id, label],
                )? == 0
                {
                    return Err(invalid("Launcher item no longer exists."));
                }
                load(connection)
            })
            .map_err(|error| error.to_string())
    }

    pub fn remove(state: &WorkspaceState, id: &str) -> Result<Vec<QuickLaunchItem>, String> {
        state
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM quick_launch_items WHERE connection_id=?1",
                    [id],
                )?;
                load(connection)
            })
            .map_err(|error| error.to_string())
    }

    pub fn reorder(
        state: &WorkspaceState,
        ids: Vec<String>,
    ) -> Result<Vec<QuickLaunchItem>, String> {
        state
            .with_connection(|connection| {
                let current = load(connection)?;
                let expected: HashSet<_> =
                    current.iter().map(|entry| &entry.connection.id).collect();
                let received: HashSet<_> = ids.iter().collect();
                if ids.len() != current.len() || received != expected {
                    return Err(invalid(
                        "Launcher changed. Refresh and try reordering again.",
                    ));
                }
                let transaction = connection.unchecked_transaction()?;
                for (position, id) in ids.iter().enumerate() {
                    transaction.execute(
                        "UPDATE quick_launch_items SET position=?2 WHERE connection_id=?1",
                        params![id, position as i64],
                    )?;
                }
                transaction.commit()?;
                load(connection)
            })
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub fn list_quick_launch_items(
    state: State<'_, WorkspaceState>,
) -> Result<Vec<QuickLaunchItem>, String> {
    QuickLauncherService::list(&state)
}
#[tauri::command]
pub fn pin_quick_launch_item(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<Vec<QuickLaunchItem>, String> {
    QuickLauncherService::pin(&state, &id)
}
#[tauri::command]
pub fn rename_quick_launch_item(
    state: State<'_, WorkspaceState>,
    id: String,
    label: Option<String>,
) -> Result<Vec<QuickLaunchItem>, String> {
    QuickLauncherService::rename(&state, &id, label)
}
#[tauri::command]
pub fn remove_quick_launch_item(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<Vec<QuickLaunchItem>, String> {
    QuickLauncherService::remove(&state, &id)
}
#[tauri::command]
pub fn reorder_quick_launch_items(
    state: State<'_, WorkspaceState>,
    ids: Vec<String>,
) -> Result<Vec<QuickLaunchItem>, String> {
    QuickLauncherService::reorder(&state, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;

    #[test]
    fn persists_workspace_pins_and_order_follows_live_connections_and_cascades_deletions() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-launcher-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .unwrap();
        state.with_connection(|connection| {
            connection.execute("INSERT INTO connection_folders(id,name) VALUES ('folder','Tools')", [])?;
            for (id, kind) in [("web", "url"), ("shell", "ssh"), ("desktop", "rdp")] {
                connection.execute("INSERT INTO connections(id,folder_id,connection_type,name,target) VALUES (?1,'folder',?2,?1,'host.test')", params![id,kind])?;
            }
            Ok(())
        }).unwrap();
        for id in ["web", "shell", "desktop"] {
            QuickLauncherService::pin(&state, id).unwrap();
        }
        assert!(QuickLauncherService::pin(&state, "missing").is_err());
        QuickLauncherService::rename(&state, "web", Some(" Dashboard ".into())).unwrap();
        let items = QuickLauncherService::pin(&state, "web").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label.as_deref(), Some("Dashboard"));
        assert!(QuickLauncherService::rename(&state, "web", Some("x".repeat(81))).is_err());
        assert!(
            QuickLauncherService::reorder(
                &state,
                vec!["web".into(), "web".into(), "desktop".into()]
            )
            .is_err()
        );
        assert!(QuickLauncherService::reorder(&state, vec!["web".into(), "shell".into()]).is_err());
        QuickLauncherService::reorder(&state, vec!["desktop".into(), "web".into(), "shell".into()])
            .unwrap();
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE connections SET name='Renamed source',target='new.test',icon='icon9' WHERE id='web'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let items = QuickLauncherService::list(&state).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["desktop", "web", "shell"]
        );
        assert_eq!(items[1].connection.name, "Renamed source");
        assert_eq!(items[1].connection.target, "new.test");
        assert_eq!(items[1].connection.icon, "icon9");
        assert_eq!(items[1].label.as_deref(), Some("Dashboard"));
        QuickLauncherService::rename(&state, "web", Some("   ".into())).unwrap();
        assert!(
            QuickLauncherService::list(&state).unwrap()[1]
                .label
                .is_none()
        );
        state.lock().unwrap();
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .unwrap();
        assert_eq!(
            QuickLauncherService::list(&state).unwrap()[0].connection.id,
            "desktop"
        );
        QuickLauncherService::remove(&state, "shell").unwrap();
        state
            .with_connection(|connection| {
                let count: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM connections WHERE id='shell'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1); // Unpin never deletes the source connection.
                connection.execute("DELETE FROM connections WHERE id='web'", [])?;
                Ok(())
            })
            .unwrap();
        assert_eq!(QuickLauncherService::list(&state).unwrap().len(), 1);
        let other_path = directory.join("other");
        std::fs::create_dir_all(&other_path).unwrap();
        let other = WorkspaceState::default();
        WorkspaceService::create(
            &other,
            other_path.to_string_lossy().into_owned(),
            "another-test-password",
        )
        .unwrap();
        assert!(QuickLauncherService::list(&other).unwrap().is_empty());
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM connection_folders WHERE id='folder'", [])?;
                Ok(())
            })
            .unwrap();
        assert!(QuickLauncherService::list(&state).unwrap().is_empty());
        state.lock().unwrap();
        other.lock().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
