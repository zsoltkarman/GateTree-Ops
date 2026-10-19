use rusqlite::{Connection, OptionalExtension};

pub(super) const LATEST_SCHEMA_VERSION: i64 = 22;

pub fn run_migrations(connection: &mut Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS workspace_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;

    let current_version: i64 = transaction
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .optional()?
        .flatten()
        .unwrap_or(0);

    if current_version < 1 {
        transaction.execute_batch(
            "
            INSERT OR IGNORE INTO workspace_metadata(key, value) VALUES ('format', 'gatetree-ops-workspace');
            INSERT INTO schema_migrations(version) VALUES (1);
            ",
        )?;
    }

    if current_version < 2 {
        transaction.execute_batch(
            "
            CREATE TABLE note_folders (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE notes (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                folder_id TEXT REFERENCES note_folders(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX notes_folder_updated_idx ON notes(folder_id, updated_at DESC);
            INSERT INTO schema_migrations(version) VALUES (2);
            ",
        )?;
    }

    if current_version < 3 {
        transaction.execute_batch(
            "
            CREATE TABLE vault_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE vault_entries (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                entry_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX vault_entries_name_idx ON vault_entries(name COLLATE NOCASE);
            INSERT INTO schema_migrations(version) VALUES (3);
            ",
        )?;
    }

    if current_version < 4 {
        transaction.execute_batch(
            "
            CREATE TABLE connection_folders (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                parent_id TEXT REFERENCES connection_folders(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE UNIQUE INDEX connection_folders_parent_name_idx
                ON connection_folders(parent_id, name COLLATE NOCASE);
            CREATE INDEX connection_folders_parent_idx ON connection_folders(parent_id);
            INSERT INTO schema_migrations(version) VALUES (4);
            ",
        )?;
    }

    if current_version < 5 {
        transaction.execute_batch(
            "
            CREATE TABLE connections (
                id TEXT PRIMARY KEY,
                folder_id TEXT NOT NULL REFERENCES connection_folders(id) ON DELETE CASCADE,
                connection_type TEXT NOT NULL CHECK(connection_type IN ('ssh', 'rdp', 'url')),
                name TEXT NOT NULL COLLATE NOCASE,
                target TEXT NOT NULL,
                port INTEGER,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE UNIQUE INDEX connections_folder_name_idx
                ON connections(folder_id, name COLLATE NOCASE);
            CREATE INDEX connections_folder_idx ON connections(folder_id);
            INSERT INTO schema_migrations(version) VALUES (5);
            ",
        )?;
    }

    if current_version < 6 {
        transaction.execute_batch(
            "
            ALTER TABLE connections ADD COLUMN tag TEXT NOT NULL DEFAULT '';
            INSERT INTO schema_migrations(version) VALUES (6);
            ",
        )?;
    }

    if current_version < 7 {
        transaction.execute_batch(
            "
            CREATE TABLE agent_activity (
                id TEXT PRIMARY KEY,
                connection_id TEXT REFERENCES connections(id) ON DELETE SET NULL,
                resource_type TEXT NOT NULL,
                name TEXT NOT NULL,
                target TEXT NOT NULL,
                status TEXT NOT NULL,
                opened_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX agent_activity_opened_idx ON agent_activity(opened_at DESC);
            INSERT INTO schema_migrations(version) VALUES (7);
            ",
        )?;
    }

    if current_version < 8 {
        transaction.execute_batch(
            "
            ALTER TABLE connections ADD COLUMN username TEXT;
            ALTER TABLE connections ADD COLUMN authentication TEXT
                CHECK(authentication IS NULL OR authentication IN ('password', 'private-key', 'keyboard-interactive', 'ssh-agent'));
            ALTER TABLE connections ADD COLUMN credential_reference TEXT
                REFERENCES vault_entries(id) ON DELETE SET NULL;
            INSERT INTO schema_migrations(version) VALUES (8);
            ",
        )?;
    }

    if current_version < 9 {
        transaction.execute_batch(
            "
            CREATE TABLE ssh_known_hosts (
                host TEXT NOT NULL COLLATE NOCASE,
                port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
                public_key TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                first_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(host, port)
            );

            INSERT INTO schema_migrations(version) VALUES (9);
            ",
        )?;
    }

    if current_version < 10 {
        transaction.execute_batch(
            "
            ALTER TABLE connections ADD COLUMN ssh_transport TEXT NOT NULL DEFAULT 'embedded'
                CHECK(ssh_transport IN ('embedded', 'openssh'));
            INSERT INTO schema_migrations(version) VALUES (10);
            ",
        )?;
    }

    if current_version < 11 {
        transaction.execute_batch(
            "
            CREATE TABLE ai_skill_libraries (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                path TEXT NOT NULL COLLATE NOCASE UNIQUE,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE ai_skills (
                id TEXT PRIMARY KEY,
                library_id TEXT NOT NULL REFERENCES ai_skill_libraries(id) ON DELETE CASCADE,
                name TEXT NOT NULL COLLATE NOCASE,
                description TEXT NOT NULL DEFAULT '',
                path TEXT NOT NULL COLLATE NOCASE UNIQUE,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX ai_skills_library_name_idx
                ON ai_skills(library_id, name COLLATE NOCASE);
            INSERT INTO schema_migrations(version) VALUES (11);
            ",
        )?;
    }

    if current_version < 12 {
        transaction.execute_batch(
            "
            CREATE TABLE deployment_tools (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                transport TEXT NOT NULL,
                definition_json TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE deployment_processes (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                definition_json TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE deployment_config_history (
                id TEXT PRIMARY KEY,
                resource_type TEXT NOT NULL CHECK(resource_type IN ('tool', 'process')),
                resource_id TEXT NOT NULL,
                definition_json TEXT NOT NULL,
                revision INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX deployment_config_history_resource_idx
                ON deployment_config_history(resource_type, resource_id, created_at DESC);
            INSERT INTO schema_migrations(version) VALUES (12);
            ",
        )?;
    }

    if current_version < 13 {
        transaction.execute_batch(
            "
            CREATE TABLE deployment_plans (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                process_name TEXT NOT NULL,
                definition_yaml TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE deployment_jobs (
                id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL REFERENCES deployment_plans(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL,
                name TEXT NOT NULL,
                driver TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE UNIQUE INDEX deployment_jobs_plan_sequence_idx
                ON deployment_jobs(plan_id, sequence);
            CREATE INDEX deployment_plans_created_idx
                ON deployment_plans(created_at DESC);
            INSERT INTO schema_migrations(version) VALUES (13);
            ",
        )?;
    }

    if current_version < 14 {
        transaction.execute_batch(
            "
            ALTER TABLE deployment_jobs ADD COLUMN host TEXT;
            INSERT INTO schema_migrations(version) VALUES (14);
            ",
        )?;
    }

    if current_version < 15 {
        transaction.execute_batch(
            "
            ALTER TABLE deployment_jobs ADD COLUMN target TEXT;
            ALTER TABLE deployment_jobs ADD COLUMN details TEXT NOT NULL DEFAULT '';
            UPDATE deployment_jobs SET target = host WHERE host IS NOT NULL;
            INSERT INTO schema_migrations(version) VALUES (15);
            ",
        )?;
    }

    if current_version < 16 {
        transaction.execute_batch(
            "
            ALTER TABLE deployment_jobs ADD COLUMN payload_json TEXT;
            INSERT INTO schema_migrations(version) VALUES (16);
            ",
        )?;
    }

    if current_version < 17 {
        transaction.execute_batch(
            "
            ALTER TABLE deployment_jobs ADD COLUMN log_output TEXT NOT NULL DEFAULT '';
            INSERT INTO schema_migrations(version) VALUES (17);
            ",
        )?;
    }

    if current_version < 18 {
        transaction.execute_batch(
            "
            ALTER TABLE deployment_plans ADD COLUMN plan_number INTEGER;
            UPDATE deployment_plans AS plan
            SET plan_number = (
                SELECT COUNT(*)
                FROM deployment_plans AS previous
                WHERE previous.created_at < plan.created_at
                   OR (previous.created_at = plan.created_at AND previous.rowid <= plan.rowid)
            );
            CREATE UNIQUE INDEX deployment_plans_number_idx
                ON deployment_plans(plan_number);
            INSERT OR IGNORE INTO workspace_metadata(key, value)
            SELECT 'deployment_plan_sequence', CAST(COALESCE(MAX(plan_number), 0) AS TEXT)
            FROM deployment_plans;
            INSERT INTO schema_migrations(version) VALUES (18);
            ",
        )?;
    }

    if current_version < 19 {
        transaction.execute_batch(
            "ALTER TABLE deployment_plans ADD COLUMN process_definition_json TEXT;
             INSERT INTO schema_migrations(version) VALUES (19);",
        )?;
    }

    if current_version < 20 {
        transaction.execute_batch(
            "CREATE TABLE quick_launch_items (
                connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
                label TEXT,
                position INTEGER NOT NULL
             );
             CREATE INDEX quick_launch_position_idx ON quick_launch_items(position);
             INSERT INTO schema_migrations(version) VALUES (20);",
        )?;
    }

    if current_version < 21 {
        transaction.execute_batch(
            "ALTER TABLE connections ADD COLUMN icon TEXT NOT NULL DEFAULT 'icon12';
            INSERT INTO schema_migrations(version) VALUES (21);",
        )?;
    }

    if current_version < 22 {
        transaction.execute_batch(
            "ALTER TABLE connection_folders ADD COLUMN credential_reference TEXT REFERENCES vault_entries(id) ON DELETE SET NULL;
             ALTER TABLE connection_folders ADD COLUMN credential_mode TEXT NOT NULL DEFAULT 'inherit' CHECK(credential_mode IN ('inherit', 'manual', 'keepass'));
             ALTER TABLE connections ADD COLUMN credential_mode TEXT NOT NULL DEFAULT 'inherit' CHECK(credential_mode IN ('inherit', 'manual', 'keepass'));
             UPDATE connections SET credential_mode = 'keepass' WHERE credential_reference IS NOT NULL;
             INSERT INTO schema_migrations(version) VALUES (22);",
        )?;
    }

    transaction.execute(
        "INSERT INTO workspace_metadata(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [LATEST_SCHEMA_VERSION.to_string()],
    )?;
    transaction.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_21_migration_preserves_existing_account_choices() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY); INSERT INTO schema_migrations VALUES(21);
            CREATE TABLE workspace_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE vault_entries(id TEXT PRIMARY KEY);
            CREATE TABLE connection_folders(id TEXT PRIMARY KEY);
            CREATE TABLE connections(id TEXT PRIMARY KEY,credential_reference TEXT REFERENCES vault_entries(id));
            INSERT INTO vault_entries VALUES('existing-account');
            INSERT INTO connections VALUES('assigned','existing-account'),('unassigned',NULL);").unwrap();
        run_migrations(&mut db).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT credential_mode FROM connections WHERE id='assigned'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "keepass"
        );
        assert_eq!(
            db.query_row(
                "SELECT credential_mode FROM connections WHERE id='unassigned'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "inherit"
        );
    }
    #[test]
    fn creates_notes_schema_and_can_run_repeatedly() {
        let mut connection = Connection::open_in_memory().expect("open database");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("enable foreign keys");

        run_migrations(&mut connection).expect("first migration");
        run_migrations(&mut connection).expect("second migration");

        for table in [
            "note_folders",
            "notes",
            "vault_settings",
            "vault_entries",
            "connection_folders",
            "connections",
            "agent_activity",
            "ssh_known_hosts",
            "ai_skill_libraries",
            "ai_skills",
            "deployment_tools",
            "deployment_processes",
            "deployment_config_history",
            "deployment_plans",
            "deployment_jobs",
            "quick_launch_items",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert!(exists, "missing table: {table}");
        }

        let version: String = connection
            .query_row(
                "SELECT value FROM workspace_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version");
        assert_eq!(version, "22");
    }
}
