use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};

use super::model::{Note, NoteFolder, NotesSnapshot};

pub struct NotesService;

impl NotesService {
    pub fn snapshot(state: &WorkspaceState) -> Result<NotesSnapshot, NotesError> {
        state
            .with_connection(|connection| {
                Ok(NotesSnapshot {
                    folders: load_folders(connection)?,
                    notes: load_notes(connection)?,
                })
            })
            .map_err(NotesError::from)
    }

    pub fn create_folder(state: &WorkspaceState, name: String) -> Result<NoteFolder, NotesError> {
        let name = validated_name(name)?;
        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO note_folders(id, name) VALUES (?1, ?2)",
                    params![id, name],
                )?;
                folder_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(NotesError::from)
    }

    pub fn rename_folder(
        state: &WorkspaceState,
        id: String,
        name: String,
    ) -> Result<NoteFolder, NotesError> {
        let name = validated_name(name)?;
        state
            .with_connection(|connection| {
                let changed = connection.execute(
                "UPDATE note_folders SET name = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![id, name],
            )?;
                if changed == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                folder_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(NotesError::from)
    }

    pub fn delete_folder(state: &WorkspaceState, id: String) -> Result<(), NotesError> {
        let note_count = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT COUNT(*) FROM notes WHERE folder_id = ?1",
                    [&id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .map_err(NotesError::from)?;
        if note_count > 0 {
            return Err(NotesError::FolderNotEmpty);
        }
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM note_folders WHERE id = ?1", [id])?;
                Ok(())
            })
            .map_err(NotesError::from)
    }

    pub fn create_note(
        state: &WorkspaceState,
        folder_id: Option<String>,
    ) -> Result<Note, NotesError> {
        let folder_id = folder_id.ok_or(NotesError::FolderRequired)?;
        let id = Uuid::new_v4().to_string();
        state.with_connection(|connection| {
            connection.execute(
                "INSERT INTO notes(id, title, body, folder_id) VALUES (?1, 'Untitled note', '', ?2)",
                params![id, folder_id],
            )?;
            note_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
        }).map_err(NotesError::from)
    }

    pub fn update_note(
        state: &WorkspaceState,
        id: String,
        title: String,
        body: String,
    ) -> Result<Note, NotesError> {
        state.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE notes SET title = ?2, body = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![id, title, body],
            )?;
            if changed == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            note_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
        }).map_err(NotesError::from)
    }

    pub fn move_note(
        state: &WorkspaceState,
        id: String,
        folder_id: Option<String>,
    ) -> Result<Note, NotesError> {
        let folder_id = folder_id.ok_or(NotesError::FolderRequired)?;
        state
            .with_connection(|connection| {
                let changed = connection.execute(
                    "UPDATE notes SET folder_id = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![id, folder_id],
                )?;
                if changed == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                note_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(NotesError::from)
    }

    pub fn delete_note(state: &WorkspaceState, id: String) -> Result<(), NotesError> {
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM notes WHERE id = ?1", [id])?;
                Ok(())
            })
            .map_err(NotesError::from)
    }
}

fn validated_name(name: String) -> Result<String, NotesError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        Err(NotesError::EmptyFolderName)
    } else {
        Ok(name)
    }
}

fn load_folders(connection: &Connection) -> rusqlite::Result<Vec<NoteFolder>> {
    let mut statement = connection.prepare(
        "SELECT id, name, created_at, updated_at FROM note_folders ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([], folder_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn load_notes(connection: &Connection) -> rusqlite::Result<Vec<Note>> {
    let mut statement = connection.prepare(
        "SELECT id, title, body, folder_id, created_at, updated_at
         FROM notes ORDER BY updated_at DESC, title COLLATE NOCASE",
    )?;
    statement
        .query_map([], note_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn folder_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<NoteFolder>> {
    connection
        .query_row(
            "SELECT id, name, created_at, updated_at FROM note_folders WHERE id = ?1",
            [id],
            folder_from_row,
        )
        .optional()
}

fn note_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<Note>> {
    connection
        .query_row(
            "SELECT id, title, body, folder_id, created_at, updated_at FROM notes WHERE id = ?1",
            [id],
            note_from_row,
        )
        .optional()
}

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteFolder> {
    Ok(NoteFolder {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn note_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        folder_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[derive(Debug, Error)]
pub enum NotesError {
    #[error("Folder name cannot be empty.")]
    EmptyFolderName,
    #[error("Create or select a folder before adding a note.")]
    FolderRequired,
    #[error("Delete the notes in this folder before deleting the folder.")]
    FolderNotEmpty,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::modules::workspace::WorkspaceService;

    use super::*;

    #[test]
    fn stores_plain_text_notes_in_the_encrypted_workspace() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("pharos-notes-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .expect("create encrypted workspace");

        let folder =
            NotesService::create_folder(&state, "Operations".to_owned()).expect("create folder");
        let note = NotesService::create_note(&state, Some(folder.id.clone())).expect("create note");
        let body = "kubectl get pods\nssh host.example.com\n------";
        NotesService::update_note(
            &state,
            note.id.clone(),
            "Runbook".to_owned(),
            body.to_owned(),
        )
        .expect("update note");

        let snapshot = NotesService::snapshot(&state).expect("load notes");
        assert_eq!(snapshot.folders.len(), 1);
        assert_eq!(snapshot.notes.len(), 1);
        assert_eq!(snapshot.notes[0].body, body);

        assert!(matches!(
            NotesService::delete_folder(&state, folder.id.clone()),
            Err(NotesError::FolderNotEmpty)
        ));
        NotesService::delete_note(&state, note.id).expect("delete note");
        NotesService::delete_folder(&state, folder.id).expect("delete empty folder");
        assert!(
            NotesService::snapshot(&state)
                .expect("final load")
                .notes
                .is_empty()
        );

        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
