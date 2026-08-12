use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{CockpitError, ConnectionProfile, Result};

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| CockpitError::Storage(e.to_string()))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let storage = Self {
            connection: Mutex::new(connection),
        };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);\n\
             CREATE TABLE IF NOT EXISTS connections (id TEXT PRIMARY KEY, profile_json TEXT NOT NULL, updated_at TEXT NOT NULL);\n\
             CREATE TABLE IF NOT EXISTS workspace_state (state_key TEXT PRIMARY KEY, payload_json TEXT NOT NULL, updated_at TEXT NOT NULL);\n\
             INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, datetime('now'));"
        )?;
        Ok(())
    }

    pub fn list_connections(&self) -> Result<Vec<ConnectionProfile>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        let mut statement = connection.prepare("SELECT profile_json FROM connections ORDER BY json_extract(profile_json, '$.name') COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let json = row?;
            Ok(serde_json::from_str(&json)?)
        })
        .collect()
    }

    pub fn get_connection(&self, id: Uuid) -> Result<Option<ConnectionProfile>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        let mut statement =
            connection.prepare("SELECT profile_json FROM connections WHERE id = ?1")?;
        let mut rows = statement.query([id.to_string()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let json: String = row.get(0)?;
        Ok(Some(serde_json::from_str(&json)?))
    }

    pub fn save_connection(&self, profile: &ConnectionProfile) -> Result<()> {
        profile.validate()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        connection.execute(
            "INSERT INTO connections(id, profile_json, updated_at) VALUES (?1, ?2, ?3)\n\
             ON CONFLICT(id) DO UPDATE SET profile_json = excluded.profile_json, updated_at = excluded.updated_at",
            params![profile.id.to_string(), serde_json::to_string(profile)?, profile.updated_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn delete_connection(&self, id: Uuid) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        connection.execute("DELETE FROM connections WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }

    pub fn load_workspace_state(&self, state_key: &str) -> Result<Option<String>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        connection
            .query_row(
                "SELECT payload_json FROM workspace_state WHERE state_key = ?1",
                [state_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_workspace_state(&self, state_key: &str, payload_json: &str) -> Result<()> {
        serde_json::from_str::<serde_json::Value>(payload_json).map_err(|error| {
            CockpitError::InvalidConfig(format!("工作区状态不是有效 JSON：{error}"))
        })?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CockpitError::Storage("本地数据库锁已损坏".into()))?;
        connection.execute(
            "INSERT INTO workspace_state(state_key, payload_json, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(state_key) DO UPDATE SET payload_json = excluded.payload_json, updated_at = excluded.updated_at",
            params![state_key, payload_json, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::TlsOptions;

    #[test]
    fn connections_round_trip_without_secrets() {
        let storage = Storage::open(":memory:").unwrap();
        let profile = ConnectionProfile {
            id: Uuid::new_v4(),
            driver_kind: crate::DatabaseKind::MySql,
            group: None,
            name: "Local".into(),
            host: "127.0.0.1".into(),
            port: 3306,
            username: "root".into(),
            database: None,
            tls: TlsOptions::default(),
            ssh: None,
            connect_timeout_secs: 5,
            query_timeout_secs: 30,
            pool_size: 5,
            read_only: false,
            production: false,
            color: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        storage.save_connection(&profile).unwrap();
        assert_eq!(storage.get_connection(profile.id).unwrap(), Some(profile));
    }

    #[test]
    fn workspace_state_round_trip() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .save_workspace_state("main", r#"{"tabs":[]}"#)
            .unwrap();
        assert_eq!(
            storage.load_workspace_state("main").unwrap().as_deref(),
            Some(r#"{"tabs":[]}"#)
        );
        assert!(storage.save_workspace_state("main", "not-json").is_err());
    }
}
