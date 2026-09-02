use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration as StdDuration, Instant, SystemTime};

use crate::config::Config;
use crate::contact_publish::apple;
use crate::error::{CrmError, Result};
use crate::jobs::{self, JobKind};
use crate::{commands, review};
use chrono::Duration;

pub fn run(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    let connection = commands::open_database(&config_path)?;
    if review::pending_migration_count(&connection)? > 0 {
        return Err(CrmError::Contacts(
            "migration review is required before the daemon can start; run `crm review`".into(),
        ));
    }
    let _lock = DaemonLock::acquire(config_path.parent().unwrap())?;
    connection.execute(
        "INSERT INTO daemon_state(id, pid, started_at, heartbeat_at, stopped_at, last_error)
         VALUES (1, ?1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL, NULL)
         ON CONFLICT(id) DO UPDATE SET pid=excluded.pid, started_at=CURRENT_TIMESTAMP,
         heartbeat_at=CURRENT_TIMESTAMP, stopped_at=NULL, last_error=NULL",
        [std::process::id()],
    )?;
    let recovered = jobs::recover_running(&connection)?;
    crate::progress::record_interrupted(&config_path, recovered);
    enqueue_initial(&connection)?;
    let mut watcher = SourceWatcher::new(&config)?;
    let mut gmail_due = Instant::now();
    let mut photos_due = Instant::now();
    loop {
        connection.execute(
            "UPDATE daemon_state SET heartbeat_at=CURRENT_TIMESTAMP WHERE id=1",
            [],
        )?;
        let changed = watcher.changed()?;
        if changed.contacts {
            jobs::enqueue(
                &connection,
                JobKind::Contacts,
                "iCloud Contacts changed",
                Duration::seconds(2),
            )?;
        }
        if changed.communications {
            jobs::enqueue(
                &connection,
                JobKind::Communications,
                "local communication store changed",
                Duration::seconds(2),
            )?;
        }
        if gmail_due.elapsed() >= StdDuration::from_secs(60) {
            jobs::enqueue(
                &connection,
                JobKind::Gmail,
                "Gmail history poll",
                Duration::zero(),
            )?;
            gmail_due = Instant::now();
        }
        if photos_due.elapsed() >= StdDuration::from_secs(300) {
            jobs::enqueue(
                &connection,
                JobKind::Photos,
                "Photos reconciliation",
                Duration::zero(),
            )?;
            photos_due = Instant::now();
        }
        while jobs::process_one(&config_path, &connection)? {}
        thread::sleep(StdDuration::from_secs(2));
    }
}

pub(crate) fn process_is_running(pid: i64) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn enqueue_initial(connection: &rusqlite::Connection) -> Result<()> {
    for (kind, reason) in [
        (JobKind::Contacts, "daemon startup"),
        (JobKind::Communications, "daemon startup"),
        (JobKind::Gmail, "daemon startup"),
        (JobKind::Photos, "daemon startup"),
    ] {
        jobs::enqueue(connection, kind, reason, Duration::zero())?;
    }
    Ok(())
}

struct Changes {
    contacts: bool,
    communications: bool,
}

struct SourceWatcher {
    contacts: Vec<PathBuf>,
    communications: Vec<PathBuf>,
    stamps: HashMap<PathBuf, Option<SystemTime>>,
}

impl SourceWatcher {
    fn new(config: &Config) -> Result<Self> {
        let mut contacts = Vec::new();
        if let (Some(path), Some(container)) = (
            config.paths.contacts.as_deref(),
            config.contact_publish.source_container.as_deref(),
        ) {
            contacts = watched_paths(&apple::container_path(path, container)?);
        }
        let communications = [
            config.paths.imessage.as_ref(),
            config.paths.whatsapp.as_ref(),
            config.paths.apple_calls.as_ref(),
            config.paths.whatsapp_calls.as_ref(),
        ]
        .into_iter()
        .flatten()
        .flat_map(|path| watched_paths(path))
        .collect();
        let mut watcher = Self {
            contacts,
            communications,
            stamps: HashMap::new(),
        };
        for path in watcher.contacts.iter().chain(&watcher.communications) {
            watcher.stamps.insert(path.clone(), modified(path)?);
        }
        Ok(watcher)
    }

    fn changed(&mut self) -> Result<Changes> {
        let contacts = changed_group(&self.contacts, &mut self.stamps)?;
        let communications = changed_group(&self.communications, &mut self.stamps)?;
        Ok(Changes {
            contacts,
            communications,
        })
    }
}

fn watched_paths(path: &Path) -> Vec<PathBuf> {
    vec![
        path.to_owned(),
        PathBuf::from(format!("{}-wal", path.display())),
    ]
}

fn changed_group(
    paths: &[PathBuf],
    stamps: &mut HashMap<PathBuf, Option<SystemTime>>,
) -> Result<bool> {
    let mut changed = false;
    for path in paths {
        let current = modified(path)?;
        if stamps.get(path) != Some(&current) {
            stamps.insert(path.clone(), current);
            changed = true;
        }
    }
    Ok(changed)
}

fn modified(path: &Path) -> Result<Option<SystemTime>> {
    match fs::metadata(path) {
        Ok(metadata) => metadata
            .modified()
            .map(Some)
            .map_err(|source| CrmError::Io {
                path: path.to_owned(),
                source,
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CrmError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

struct DaemonLock(PathBuf);

impl DaemonLock {
    fn acquire(directory: &Path) -> Result<Self> {
        let path = directory.join("daemon.lock");
        if let Ok(pid) = fs::read_to_string(&path) {
            let alive = pid.trim().parse().is_ok_and(process_is_running);
            if alive {
                return Err(CrmError::InvalidConfig(format!(
                    "CRM daemon is already running as PID {}",
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

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
