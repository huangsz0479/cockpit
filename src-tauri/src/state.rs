use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use cockpit_core::{DriverSession, Result, Storage};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::secrets::SecretStore;

pub type TabSession = (Uuid, Arc<dyn DriverSession>);

pub struct AppState {
    pub storage: Storage,
    pub secrets: SecretStore,
    pub sessions: RwLock<HashMap<Uuid, Arc<dyn DriverSession>>>,
    pub tab_sessions: RwLock<HashMap<Uuid, TabSession>>,
    pub transfers: RwLock<HashMap<Uuid, CancellationToken>>,
    pub log_dir: PathBuf,
}

impl AppState {
    pub fn new(storage: Storage, data_dir: &Path, log_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            storage,
            secrets: SecretStore::new(data_dir)?,
            sessions: RwLock::new(HashMap::new()),
            tab_sessions: RwLock::new(HashMap::new()),
            transfers: RwLock::new(HashMap::new()),
            log_dir,
        })
    }
}
