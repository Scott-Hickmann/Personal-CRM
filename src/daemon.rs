use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration as StdDuration, Instant, SystemTime};

use crate::config::Config;
use crate::contact_publish::apple;
use crate::coordinator::{self, WorkKind};
use crate::error::{CrmError, Result};
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
    let _lock = coordinator::WriterLock::acquire(config_path.parent().unwrap())?;
    connection.execute(
        "INSERT INTO daemon_state(id, pid, started_at, heartbeat_at, stopped_at, last_error)
         VALUES (1, ?1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL, NULL)
         ON CONFLICT(id) DO UPDATE SET pid=excluded.pid, started_at=CURRENT_TIMESTAMP,
         heartbeat_at=CURRENT_TIMESTAMP, stopped_at=NULL, last_error=NULL",
        [std::process::id()],
    )?;
    let recovered = coordinator::recover_interrupted(&connection)?;
    crate::progress::record_interrupted(&config_path, recovered);
    enqueue_initial(&connection)?;
    let mut watcher = SourceWatcher::new(&config)?;
    let mut gmail_due = Instant::now();
    let mut photos_due = Instant::now();
    let mut audit_due = Instant::now();
    loop {
        connection.execute(
            "UPDATE daemon_state SET heartbeat_at=CURRENT_TIMESTAMP WHERE id=1",
            [],
        )?;
        let changed = watcher.changed()?;
        if changed.contacts {
            coordinator::request(
                &connection,
                WorkKind::Contacts,
                "iCloud Contacts changed",
                Duration::seconds(2),
            )?;
        }
        for (changed, kind, reason) in [
            (
                changed.imessage,
                WorkKind::Imessage,
                "iMessage store changed",
            ),
            (
                changed.whatsapp,
                WorkKind::Whatsapp,
                "WhatsApp store changed",
            ),
            (
                changed.apple_calls,
                WorkKind::AppleCalls,
                "Apple call store changed",
            ),
            (
                changed.whatsapp_calls,
                WorkKind::WhatsappCalls,
                "WhatsApp call store changed",
            ),
            (changed.photos, WorkKind::Photos, "Photos store changed"),
        ] {
            if changed {
                coordinator::request(&connection, kind, reason, Duration::seconds(2))?;
            }
        }
        if gmail_due.elapsed() >= StdDuration::from_secs(60) {
            coordinator::request(
                &connection,
                WorkKind::Gmail,
                "Gmail history poll",
                Duration::zero(),
            )?;
            gmail_due = Instant::now();
        }
        if photos_due.elapsed() >= StdDuration::from_secs(300) {
            coordinator::request(
                &connection,
                WorkKind::Photos,
                "Photos reconciliation",
                Duration::zero(),
            )?;
            photos_due = Instant::now();
        }
        if audit_due.elapsed() >= StdDuration::from_secs(86_400) {
            for kind in [
                WorkKind::Imessage,
                WorkKind::Whatsapp,
                WorkKind::AppleCalls,
                WorkKind::WhatsappCalls,
            ] {
                coordinator::request(&connection, kind, "daily deletion audit", Duration::zero())?;
            }
            audit_due = Instant::now();
        }
        if !coordinator::process_one(&config_path, &connection)? {
            std::thread::sleep(StdDuration::from_millis(500));
        }
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
        (WorkKind::Contacts, "daemon startup"),
        (WorkKind::Imessage, "daemon startup"),
        (WorkKind::Whatsapp, "daemon startup"),
        (WorkKind::AppleCalls, "daemon startup"),
        (WorkKind::WhatsappCalls, "daemon startup"),
        (WorkKind::Gmail, "daemon startup"),
        (WorkKind::Photos, "daemon startup"),
    ] {
        coordinator::request(connection, kind, reason, Duration::zero())?;
    }
    Ok(())
}

struct Changes {
    contacts: bool,
    imessage: bool,
    whatsapp: bool,
    apple_calls: bool,
    whatsapp_calls: bool,
    photos: bool,
}

struct SourceWatcher {
    contacts: Vec<PathBuf>,
    imessage: Vec<PathBuf>,
    whatsapp: Vec<PathBuf>,
    apple_calls: Vec<PathBuf>,
    whatsapp_calls: Vec<PathBuf>,
    photos: Vec<PathBuf>,
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
        let watched = |path: Option<&PathBuf>| {
            path.into_iter()
                .flat_map(|path| watched_paths(path))
                .collect()
        };
        let photos = crate::photos_library::discover_library(None)
            .ok()
            .map(|library| watched_paths(&library.join("database/Photos.sqlite")))
            .unwrap_or_default();
        let mut watcher = Self {
            contacts,
            imessage: watched(config.paths.imessage.as_ref()),
            whatsapp: watched(config.paths.whatsapp.as_ref()),
            apple_calls: watched(config.paths.apple_calls.as_ref()),
            whatsapp_calls: watched(config.paths.whatsapp_calls.as_ref()),
            photos,
            stamps: HashMap::new(),
        };
        let paths: Vec<_> = watcher.paths().cloned().collect();
        for path in paths {
            watcher.stamps.insert(path.clone(), modified(&path)?);
        }
        Ok(watcher)
    }

    fn changed(&mut self) -> Result<Changes> {
        Ok(Changes {
            contacts: changed_group(&self.contacts, &mut self.stamps)?,
            imessage: changed_group(&self.imessage, &mut self.stamps)?,
            whatsapp: changed_group(&self.whatsapp, &mut self.stamps)?,
            apple_calls: changed_group(&self.apple_calls, &mut self.stamps)?,
            whatsapp_calls: changed_group(&self.whatsapp_calls, &mut self.stamps)?,
            photos: changed_group(&self.photos, &mut self.stamps)?,
        })
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.contacts
            .iter()
            .chain(&self.imessage)
            .chain(&self.whatsapp)
            .chain(&self.apple_calls)
            .chain(&self.whatsapp_calls)
            .chain(&self.photos)
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
