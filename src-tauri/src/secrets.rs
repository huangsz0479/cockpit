use std::{collections::HashMap, path::Path, sync::RwLock};

#[cfg(debug_assertions)]
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

#[cfg(all(debug_assertions, unix))]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use cockpit_core::{CockpitError, Result};
use uuid::Uuid;

#[cfg(not(debug_assertions))]
const SERVICE: &str = "com.cockpit.db";
// Debug binaries get a new macOS code signature after rebuilds, so Keychain would
// repeatedly request access. Keep this explicitly development-only.
#[cfg(debug_assertions)]
const DEV_SECRETS_FILE: &str = "dev-secrets.json";

pub struct SecretStore {
    session_values: RwLock<HashMap<String, String>>,
    #[cfg(debug_assertions)]
    dev_file: PathBuf,
}

impl SecretStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        #[cfg(debug_assertions)]
        {
            let dev_file = data_dir.join(DEV_SECRETS_FILE);
            let session_values = match fs::read(&dev_file) {
                Ok(contents) => serde_json::from_slice(&contents)
                    .map_err(|error| CockpitError::SecretStore(error.to_string()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
                Err(error) => return Err(CockpitError::SecretStore(error.to_string())),
            };
            Ok(Self {
                session_values: RwLock::new(session_values),
                dev_file,
            })
        }

        #[cfg(not(debug_assertions))]
        {
            let _ = data_dir;
            Ok(Self {
                session_values: RwLock::new(HashMap::new()),
            })
        }
    }

    fn account(connection_id: Uuid, key: &str) -> String {
        format!("{connection_id}:{key}")
    }

    pub fn set(&self, connection_id: Uuid, key: &str, value: &str) -> Result<bool> {
        let account = Self::account(connection_id, key);
        let mut values = self
            .session_values
            .write()
            .map_err(|_| CockpitError::SecretStore("会话凭据锁已损坏".into()))?;
        values.insert(account.clone(), value.to_string());

        #[cfg(debug_assertions)]
        let persisted = self.persist_dev_values(&values).is_ok();

        #[cfg(not(debug_assertions))]
        let persisted = keyring::Entry::new(SERVICE, &account)
            .and_then(|entry| entry.set_password(value))
            .is_ok();

        Ok(persisted)
    }

    pub fn get(&self, connection_id: Uuid, key: &str) -> Result<Option<String>> {
        let account = Self::account(connection_id, key);
        if let Some(value) = self
            .session_values
            .read()
            .map_err(|_| CockpitError::SecretStore("会话凭据锁已损坏".into()))?
            .get(&account)
        {
            return Ok(Some(value.clone()));
        }
        #[cfg(debug_assertions)]
        {
            Ok(None)
        }

        #[cfg(not(debug_assertions))]
        {
            match keyring::Entry::new(SERVICE, &account).and_then(|entry| entry.get_password()) {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(CockpitError::SecretStore(error.to_string())),
            }
        }
    }

    pub fn contains(&self, connection_id: Uuid, key: &str) -> Result<bool> {
        self.get(connection_id, key).map(|value| value.is_some())
    }

    pub fn delete_connection(&self, connection_id: Uuid) {
        #[cfg(debug_assertions)]
        if let Ok(mut values) = self.session_values.write() {
            for key in ["mysql_password", "ssh_password", "ssh_key_passphrase"] {
                values.remove(&Self::account(connection_id, key));
            }
            let _ = self.persist_dev_values(&values);
        }

        #[cfg(not(debug_assertions))]
        for key in ["mysql_password", "ssh_password", "ssh_key_passphrase"] {
            let account = Self::account(connection_id, key);
            let _ =
                keyring::Entry::new(SERVICE, &account).and_then(|entry| entry.delete_credential());
            if let Ok(mut values) = self.session_values.write() {
                values.remove(&account);
            }
        }
    }

    #[cfg(debug_assertions)]
    fn persist_dev_values(&self, values: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.dev_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| CockpitError::SecretStore(error.to_string()))?;
        }
        let contents = serde_json::to_vec(values)
            .map_err(|error| CockpitError::SecretStore(error.to_string()))?;
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&self.dev_file)
            .map_err(|error| CockpitError::SecretStore(error.to_string()))?;
        file.write_all(&contents)
            .map_err(|error| CockpitError::SecretStore(error.to_string()))?;
        #[cfg(unix)]
        fs::set_permissions(&self.dev_file, fs::Permissions::from_mode(0o600))
            .map_err(|error| CockpitError::SecretStore(error.to_string()))?;
        Ok(())
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn debug_secrets_persist_without_keychain() {
        let directory = std::env::temp_dir().join(format!("cockpit-secrets-{}", Uuid::new_v4()));
        let connection_id = Uuid::new_v4();

        let store = SecretStore::new(&directory).unwrap();
        assert!(
            store
                .set(connection_id, "mysql_password", "secret")
                .unwrap()
        );
        drop(store);

        let store = SecretStore::new(&directory).unwrap();
        assert_eq!(
            store.get(connection_id, "mysql_password").unwrap(),
            Some("secret".into())
        );
        store.delete_connection(connection_id);
        drop(store);

        let store = SecretStore::new(&directory).unwrap();
        assert_eq!(store.get(connection_id, "mysql_password").unwrap(), None);

        let _ = fs::remove_dir_all(directory);
    }
}
