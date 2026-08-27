use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::RwLock,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use cockpit_core::{CockpitError, Result};
use rand_core::{OsRng, RngCore};
use uuid::Uuid;

/// 设备密钥文件：加密本机存储的全部凭据，属于绝不能被外部写命令覆盖的
/// 应用私密文件（路径守卫使用，见 commands.rs）。
pub(crate) const DEVICE_KEY_FILE: &str = "device.key";
/// 加密的凭据库文件（同上，受写入守卫保护）。
pub(crate) const SECRETS_VAULT_FILE: &str = "credentials.vault";
const VAULT_MAGIC: &[u8; 8] = b"CKPVAULT";
const NONCE_LENGTH: usize = 12;
const KEY_LENGTH: usize = 32;

pub struct SecretStore {
    session_values: RwLock<HashMap<String, String>>,
    device_key: [u8; KEY_LENGTH],
    vault_file: PathBuf,
}

impl SecretStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir).map_err(secret_store_error)?;
        let device_key = load_or_create_device_key(&data_dir.join(DEVICE_KEY_FILE))?;
        let vault_file = data_dir.join(SECRETS_VAULT_FILE);
        let session_values = load_vault(&vault_file, &device_key)?;
        Ok(Self {
            session_values: RwLock::new(session_values),
            device_key,
            vault_file,
        })
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
        let previous = values.insert(account.clone(), value.to_string());
        if let Err(error) = persist_vault(&self.vault_file, &self.device_key, &values) {
            // Roll back the in-memory change so the store never claims a
            // password exists that was not persisted to disk.
            match previous {
                Some(previous) => {
                    values.insert(account, previous);
                }
                None => {
                    values.remove(&account);
                }
            }
            return Err(error);
        }
        Ok(true)
    }

    pub fn get(&self, connection_id: Uuid, key: &str) -> Result<Option<String>> {
        let account = Self::account(connection_id, key);
        Ok(self
            .session_values
            .read()
            .map_err(|_| CockpitError::SecretStore("会话凭据锁已损坏".into()))?
            .get(&account)
            .cloned())
    }

    pub fn contains(&self, connection_id: Uuid, key: &str) -> Result<bool> {
        self.get(connection_id, key).map(|value| value.is_some())
    }

    pub fn delete_connection(&self, connection_id: Uuid) {
        if let Ok(mut values) = self.session_values.write() {
            for key in ["mysql_password", "ssh_password", "ssh_key_passphrase"] {
                values.remove(&Self::account(connection_id, key));
            }
            if let Err(error) = persist_vault(&self.vault_file, &self.device_key, &values) {
                log::warn!(
                    "failed to persist credential vault after deleting connection {connection_id}: {error}"
                );
            }
        }
    }
}

fn secret_store_error(error: impl std::fmt::Display) -> CockpitError {
    CockpitError::SecretStore(error.to_string())
}

fn secure_file(path: &Path) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path).map_err(secret_store_error)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(secret_store_error)?;
    Ok(file)
}

fn load_or_create_device_key(path: &Path) -> Result<[u8; KEY_LENGTH]> {
    match fs::read(path) {
        Ok(contents) => contents.try_into().map_err(|_| {
            CockpitError::SecretStore("本机凭据密钥格式无效，无法解密已保存的密码".into())
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0_u8; KEY_LENGTH];
            OsRng.fill_bytes(&mut key);
            let mut file = secure_file(path)?;
            file.write_all(&key).map_err(secret_store_error)?;
            file.sync_all().map_err(secret_store_error)?;
            Ok(key)
        }
        Err(error) => Err(secret_store_error(error)),
    }
}

fn load_vault(path: &Path, key: &[u8; KEY_LENGTH]) -> Result<HashMap<String, String>> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(secret_store_error(error)),
    };
    if contents.len() <= VAULT_MAGIC.len() + NONCE_LENGTH
        || &contents[..VAULT_MAGIC.len()] != VAULT_MAGIC
    {
        return Err(CockpitError::SecretStore("本机凭据库格式无效".into()));
    }
    let nonce_start = VAULT_MAGIC.len();
    let encrypted_start = nonce_start + NONCE_LENGTH;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(secret_store_error)?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&contents[nonce_start..encrypted_start]),
            &contents[encrypted_start..],
        )
        .map_err(|_| CockpitError::SecretStore("本机凭据库无法解密或已损坏".into()))?;
    serde_json::from_slice(&plaintext).map_err(secret_store_error)
}

fn persist_vault(
    path: &Path,
    key: &[u8; KEY_LENGTH],
    values: &HashMap<String, String>,
) -> Result<()> {
    let plaintext = serde_json::to_vec(values).map_err(secret_store_error)?;
    let mut nonce = [0_u8; NONCE_LENGTH];
    OsRng.fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(secret_store_error)?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(secret_store_error)?;
    let mut file = secure_file(path)?;
    file.write_all(VAULT_MAGIC).map_err(secret_store_error)?;
    file.write_all(&nonce).map_err(secret_store_error)?;
    file.write_all(&encrypted).map_err(secret_store_error)?;
    file.sync_all().map_err(secret_store_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_secrets_persist_without_keychain() {
        let directory = std::env::temp_dir().join(format!("cockpit-secrets-{}", Uuid::new_v4()));
        let connection_id = Uuid::new_v4();

        let store = SecretStore::new(&directory).unwrap();
        assert!(
            store
                .set(connection_id, "mysql_password", "secret")
                .unwrap()
        );
        drop(store);

        let vault = fs::read(directory.join(SECRETS_VAULT_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&vault).contains("secret"));
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(directory.join(DEVICE_KEY_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(directory.join(SECRETS_VAULT_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

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

    #[test]
    fn invalid_device_key_is_not_silently_replaced() {
        let directory = std::env::temp_dir().join(format!("cockpit-secrets-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(DEVICE_KEY_FILE), b"invalid").unwrap();

        let error = SecretStore::new(&directory).err().unwrap();
        assert!(error.to_string().contains("密钥格式无效"));

        let _ = fs::remove_dir_all(directory);
    }
}
