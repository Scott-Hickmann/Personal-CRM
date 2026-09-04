use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{CrmError, Result};

pub(crate) struct WriterLock(PathBuf);

impl WriterLock {
    pub(crate) fn acquire(directory: &Path) -> Result<Self> {
        let path = directory.join("coordinator.lock");
        if let Ok(pid) = fs::read_to_string(&path) {
            let alive = pid
                .trim()
                .parse()
                .is_ok_and(crate::daemon::process_is_running);
            if alive {
                return Err(CrmError::InvalidConfig(format!(
                    "CRM coordinator is already running as PID {}",
                    pid.trim()
                )));
            }
            fs::remove_file(&path).map_err(|source| CrmError::Io {
                path: path.clone(),
                source,
            })?;
        }
        use std::io::Write;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| CrmError::Io {
                path: path.clone(),
                source,
            })?;
        writeln!(file, "{}", std::process::id()).map_err(|source| CrmError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self(path))
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
