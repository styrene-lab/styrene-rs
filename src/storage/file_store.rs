use std::fs;
use std::path::{Path, PathBuf};

use crate::error::LxmfError;
use crate::message::WireMessage;
use crate::storage::Store;

pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

impl Store for FileStore {
    fn save(&self, msg: &WireMessage) -> Result<(), LxmfError> {
        let id = msg.message_id();
        let path = self.root.join(hex::encode(id));
        fs::write(path, msg.pack().map_err(|_| LxmfError::Unimplemented)?)
            .map_err(|_| LxmfError::Unimplemented)
    }

    fn get(&self, id: &[u8; 32]) -> Result<WireMessage, LxmfError> {
        let path = self.root.join(hex::encode(id));
        let bytes = fs::read(path).map_err(|_| LxmfError::Unimplemented)?;
        WireMessage::unpack(&bytes).map_err(|_| LxmfError::Unimplemented)
    }
}
