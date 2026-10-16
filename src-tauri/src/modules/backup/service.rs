use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rusqlite::{
    Connection, params_from_iter,
    types::{Value, ValueRef},
};
use thiserror::Error;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};

use super::model::{
    BACKUP_FORMAT, BACKUP_VERSION, BackupDocument, BackupSummary, BackupValue, TableBackup,
};

const TECHNICAL_TABLES: &[&str] = &["schema_migrations", "workspace_metadata"];

pub struct BackupService;

impl BackupService {
    pub fn export_json(state: &WorkspaceState) -> Result<(String, BackupSummary), BackupError> {
        let document = state
            .with_connection(load_document)
            .map_err(BackupError::from)?;
        let summary = summary(&document);
        let json = serde_json::to_string_pretty(&document)?;
        Ok((json, summary))
    }

    pub fn import_json(state: &WorkspaceState, json: &str) -> Result<BackupSummary, BackupError> {
        let document: BackupDocument = serde_json::from_str(json)?;
        let current_schema = state
            .with_connection(load_schema)
            .map_err(BackupError::from)?;
        let prepared = validate_and_prepare(&document, &current_schema)?;
        let result = summary(&document);
        state
            .with_connection(|connection| replace_content(connection, &current_schema, &prepared))
            .map_err(BackupError::from)?;
        Ok(result)
    }
}

fn load_document(connection: &Connection) -> rusqlite::Result<BackupDocument> {
    let schema = load_schema(connection)?;
    let schema_version = connection.query_row(
        "SELECT value FROM workspace_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    let mut tables = BTreeMap::new();

    for (table_name, columns) in schema {
        let selected_columns = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {selected_columns} FROM {}",
            quote_identifier(&table_name)
        );
        let mut statement = connection.prepare(&sql)?;
        let mut query = statement.query([])?;
        let mut rows = Vec::new();
        while let Some(row) = query.next()? {
            let mut values = Vec::with_capacity(columns.len());
            for index in 0..columns.len() {
                values.push(backup_value(row.get_ref(index)?));
            }
            rows.push(values);
        }
        tables.insert(table_name, TableBackup { columns, rows });
    }

    Ok(BackupDocument {
        format: BACKUP_FORMAT.to_owned(),
        version: BACKUP_VERSION,
        schema_version,
        tables,
    })
}

fn load_schema(connection: &Connection) -> rusqlite::Result<BTreeMap<String, Vec<String>>> {
    let mut table_statement = connection.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let table_names = table_statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut schema = BTreeMap::new();

    for table_name in table_names {
        if TECHNICAL_TABLES.contains(&table_name.as_str()) {
            continue;
        }
        let sql = format!("PRAGMA table_info({})", quote_identifier(&table_name));
        let mut column_statement = connection.prepare(&sql)?;
        let columns = column_statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        schema.insert(table_name, columns);
    }
    Ok(schema)
}

fn validate_and_prepare(
    document: &BackupDocument,
    current_schema: &BTreeMap<String, Vec<String>>,
) -> Result<BTreeMap<String, PreparedTable>, BackupError> {
    if document.format != BACKUP_FORMAT {
        return Err(BackupError::InvalidFormat);
    }
    if document.version != BACKUP_VERSION {
        return Err(BackupError::UnsupportedVersion(document.version));
    }

    let mut prepared = BTreeMap::new();
    for (table_name, table) in &document.tables {
        let expected_columns = current_schema
            .get(table_name)
            .ok_or_else(|| BackupError::UnknownTable(table_name.clone()))?;
        if &table.columns != expected_columns {
            return Err(BackupError::SchemaMismatch(table_name.clone()));
        }

        let mut rows = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            if row.len() != table.columns.len() {
                return Err(BackupError::InvalidRow(table_name.clone()));
            }
            rows.push(
                row.iter()
                    .map(database_value)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        prepared.insert(
            table_name.clone(),
            PreparedTable {
                columns: table.columns.clone(),
                rows,
            },
        );
    }
    Ok(prepared)
}

fn replace_content(
    connection: &Connection,
    current_schema: &BTreeMap<String, Vec<String>>,
    tables: &BTreeMap<String, PreparedTable>,
) -> rusqlite::Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch("PRAGMA defer_foreign_keys = ON")?;

    for table_name in current_schema.keys() {
        transaction.execute(&format!("DELETE FROM {}", quote_identifier(table_name)), [])?;
    }

    for (table_name, table) in tables {
        if table.rows.is_empty() {
            continue;
        }
        let columns = table
            .columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=table.columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({columns}) VALUES ({placeholders})",
            quote_identifier(table_name)
        );
        let mut statement = transaction.prepare(&sql)?;
        for row in &table.rows {
            statement.execute(params_from_iter(row.iter()))?;
        }
    }
    transaction.commit()
}

fn backup_value(value: ValueRef<'_>) -> BackupValue {
    match value {
        ValueRef::Null => BackupValue::Null,
        ValueRef::Integer(value) => BackupValue::Integer(value),
        ValueRef::Real(value) => BackupValue::Real(value),
        ValueRef::Text(value) => BackupValue::Text(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => BackupValue::Blob(BASE64.encode(value)),
    }
}

fn database_value(value: &BackupValue) -> Result<Value, BackupError> {
    Ok(match value {
        BackupValue::Null => Value::Null,
        BackupValue::Integer(value) => Value::Integer(*value),
        BackupValue::Real(value) => Value::Real(*value),
        BackupValue::Text(value) => Value::Text(value.clone()),
        BackupValue::Blob(value) => {
            Value::Blob(BASE64.decode(value).map_err(|_| BackupError::InvalidBlob)?)
        }
    })
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn summary(document: &BackupDocument) -> BackupSummary {
    BackupSummary {
        tables: document.tables.len(),
        records: document.tables.values().map(|table| table.rows.len()).sum(),
    }
}

struct PreparedTable {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("This is not a PHAROS backup file.")]
    InvalidFormat,
    #[error("PHAROS backup version {0} is not supported.")]
    UnsupportedVersion(u32),
    #[error("The backup contains the unknown table '{0}'. Update PHAROS before importing it.")]
    UnknownTable(String),
    #[error("The backup schema for table '{0}' does not match this PHAROS version.")]
    SchemaMismatch(String),
    #[error("The backup contains an invalid row in table '{0}'.")]
    InvalidRow(String),
    #[error("The backup contains invalid binary data.")]
    InvalidBlob,
    #[error("Backup JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn automatically_exports_and_restores_new_content_tables() {
        let directory = std::env::temp_dir().join(format!("pharos-backup-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .expect("create workspace");

        state
            .with_connection(|connection| {
                connection.execute_batch(
                    "CREATE TABLE future_module_content (
                        id INTEGER PRIMARY KEY,
                        title TEXT NOT NULL,
                        payload BLOB
                    );
                    INSERT INTO future_module_content(title, payload)
                    VALUES ('automatic backup', X'001122');",
                )
            })
            .expect("create future module data");

        let (json, exported) = BackupService::export_json(&state).expect("export backup");
        assert!(json.contains("future_module_content"));
        assert!(exported.tables >= 5);

        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM future_module_content", [])?;
                Ok(())
            })
            .expect("clear future module data");
        let imported = BackupService::import_json(&state, &json).expect("import backup");
        assert_eq!(imported.records, exported.records);

        let restored: (String, Vec<u8>) = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT title, payload FROM future_module_content",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .expect("read restored data");
        assert_eq!(restored.0, "automatic backup");
        assert_eq!(restored.1, vec![0x00, 0x11, 0x22]);

        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
