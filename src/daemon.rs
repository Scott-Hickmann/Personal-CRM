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
    let mut audit_due = Instant::now();
    let mut workers = HashMap::new();
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
        for (changed, kind, reason) in [
            (
                changed.imessage,
                JobKind::Imessage,
                "iMessage store changed",
            ),
            (
                changed.whatsapp,
                JobKind::Whatsapp,
                "WhatsApp store changed",
            ),
            (
                changed.apple_calls,
                JobKind::AppleCalls,
                "Apple call store changed",
            ),
            (
                changed.whatsapp_calls,
                JobKind::WhatsappCalls,
                "WhatsApp call store changed",
            ),
            (changed.photos, JobKind::Photos, "Photos store changed"),
        ] {
            if changed {
                jobs::enqueue(&connection, kind, reason, Duration::seconds(2))?;
            }
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
        if audit_due.elapsed() >= StdDuration::from_secs(86_400) {
            for kind in [
                JobKind::Imessage,
                JobKind::Whatsapp,
                JobKind::AppleCalls,
                JobKind::WhatsappCalls,
            ] {
                jobs::enqueue(&connection, kind, "daily deletion audit", Duration::zero())?;
            }
            audit_due = Instant::now();
        }
        reap_workers(&connection, &mut workers)?;
        for (id, kind) in jobs::ready(&connection)? {
            if workers.contains_key(&kind) {
                continue;
            }
            let worker_config = config_path.clone();
            workers.insert(
                kind,
                thread::spawn(move || (id, jobs::process(&worker_config, id))),
            );
        }
        thread::sleep(StdDuration::from_millis(500));
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
        (JobKind::Imessage, "daemon startup"),
        (JobKind::Whatsapp, "daemon startup"),
        (JobKind::AppleCalls, "daemon startup"),
        (JobKind::WhatsappCalls, "daemon startup"),
        (JobKind::Gmail, "daemon startup"),
        (JobKind::Photos, "daemon startup"),
    ] {
        jobs::enqueue(connection, kind, reason, Duration::zero())?;
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

type Worker = thread::JoinHandle<(i64, Result<bool>)>;

fn reap_workers(
    connection: &rusqlite::Connection,
    workers: &mut HashMap<JobKind, Worker>,
) -> Result<()> {
    let finished: Vec<_> = workers
        .iter()
        .filter(|(_, worker)| worker.is_finished())
        .map(|(kind, _)| *kind)
        .collect();
    for kind in finished {
        let worker = workers.remove(&kind).unwrap();
        match worker.join() {
            Ok((_, Ok(_))) => {}
            Ok((id, Err(error))) => {
                jobs::recover_job(connection, id, &error.to_string())?;
                eprintln!("{} worker failed: {error}", kind.as_str());
            }
            Err(_) => {
                jobs::recover_kind(connection, kind, "worker panicked")?;
                eprintln!("{} worker panicked", kind.as_str());
            }
        }
    }
    Ok(())
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
