mod runner;

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Contacts,
    Imessage,
    Whatsapp,
    AppleCalls,
    WhatsappCalls,
    Gmail,
    Scoring,
    Photos,
    GooglePublish,
    Suggestions,
}

impl WorkKind {
    pub(crate) const SOURCES: [Self; 6] = [
        Self::Contacts,
        Self::Imessage,
        Self::Whatsapp,
        Self::AppleCalls,
        Self::WhatsappCalls,
        Self::Gmail,
    ];
    pub(crate) const MAINTENANCE: [Self; 4] = [
        Self::Scoring,
        Self::Photos,
        Self::GooglePublish,
        Self::Suggestions,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contacts => "contacts",
            Self::Imessage => "imessage",
            Self::Whatsapp => "whatsapp",
            Self::AppleCalls => "apple_calls",
            Self::WhatsappCalls => "whatsapp_calls",
            Self::Gmail => "gmail",
            Self::Scoring => "scoring",
            Self::Photos => "photos",
            Self::GooglePublish => "google_publish",
            Self::Suggestions => "suggestions",
        }
    }

    pub(crate) fn is_source(self) -> bool {
        Self::SOURCES.contains(&self)
    }
}

pub(crate) fn request(
    connection: &Connection,
    kind: WorkKind,
    reason: &str,
    delay: Duration,
) -> Result<i64> {
    let table = table(kind);
    let run_after = (Utc::now() + delay).to_rfc3339();
    connection.execute(
        &format!(
            "INSERT INTO {table}(kind, state, reason, run_after, requested_generation)
             VALUES (?1, 'pending', ?2, ?3, 1)
             ON CONFLICT(kind) DO UPDATE SET
               requested_generation={table}.requested_generation+1,
               state=CASE WHEN {table}.state='running' THEN 'running' ELSE 'pending' END,
               attempts=CASE WHEN {table}.state='failed' THEN 0 ELSE {table}.attempts END,
               reason=excluded.reason,
               run_after=CASE WHEN {table}.state='running' THEN {table}.run_after
                              ELSE MIN({table}.run_after, excluded.run_after) END,
               error=NULL, updated_at=CURRENT_TIMESTAMP"
        ),
        params![kind.as_str(), reason, run_after],
    )?;
    connection
        .query_row(
            &format!("SELECT requested_generation FROM {table} WHERE kind=?1"),
            [kind.as_str()],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn recover_interrupted(connection: &Connection) -> Result<usize> {
    let mut recovered = 0;
    for table in ["source_sync_state", "maintenance_state"] {
        recovered += connection.execute(
            &format!(
                "UPDATE {table} SET state='pending', run_after=?1,
                 error='daemon restarted while work was running', updated_at=CURRENT_TIMESTAMP
                 WHERE state='running'"
            ),
            [Utc::now().to_rfc3339()],
        )?;
    }
    Ok(recovered)
}

pub(crate) fn process_one(config_path: &Path, connection: &Connection) -> Result<bool> {
    let now = Utc::now().to_rfc3339();
    for kind in WorkKind::SOURCES.into_iter().chain(WorkKind::MAINTENANCE) {
        let ready: bool = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE kind=?1 AND state='pending' AND run_after<=?2)",
                table(kind)
            ),
            params![kind.as_str(), now],
            |row| row.get(0),
        )?;
        if ready {
            if runner::process(config_path, connection, kind)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(crate) fn run_now(config_path: &Path, kind: WorkKind) -> Result<()> {
    let _lock = WriterLock::acquire(config_path.parent().unwrap())?;
    let connection = crate::commands::open_database(config_path)?;
    if kind == WorkKind::Scoring {
        crate::scoring::mark_all_dirty(&connection, "manual scoring run")?;
    }
    let generation = request(&connection, kind, "manual run requested", Duration::zero())?;
    loop {
        if completed(&connection, kind, generation)? {
            return Ok(());
        }
        if !process_one(config_path, &connection)? {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

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

pub(crate) fn completed(connection: &Connection, kind: WorkKind, generation: i64) -> Result<bool> {
    let state: Option<(String, i64, Option<String>)> = connection
        .query_row(
            &format!(
                "SELECT state, completed_generation, error FROM {} WHERE kind=?1",
                table(kind)
            ),
            [kind.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match state {
        Some((state, _, error)) if state == "failed" => Err(CrmError::InvalidConfig(
            error.unwrap_or_else(|| format!("{} failed", kind.as_str())),
        )),
        Some((_, completed, _)) => Ok(completed >= generation),
        None => Ok(false),
    }
}

pub(crate) fn pending_count(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM source_sync_state WHERE state='pending') +
               (SELECT COUNT(*) FROM maintenance_state WHERE state='pending')",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn failed_count(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM source_sync_state WHERE state='failed') +
               (SELECT COUNT(*) FROM maintenance_state WHERE state='failed')",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn running(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT kind FROM source_sync_state WHERE state='running'
         UNION ALL SELECT kind FROM maintenance_state WHERE state='running' ORDER BY kind",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(crate) fn table(kind: WorkKind) -> &'static str {
    if kind.is_source() {
        "source_sync_state"
    } else {
        "maintenance_state"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (tempfile::TempDir, Connection) {
        let directory = tempfile::tempdir().unwrap();
        let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
        (directory, connection)
    }

    #[test]
    fn requests_coalesce_into_generations_instead_of_rows() {
        let (_directory, connection) = database();
        assert_eq!(
            request(&connection, WorkKind::Gmail, "first", Duration::zero()).unwrap(),
            1
        );
        assert_eq!(
            request(&connection, WorkKind::Gmail, "second", Duration::zero()).unwrap(),
            2
        );
        let state: (i64, i64, String) = connection
            .query_row(
                "SELECT COUNT(*), requested_generation, state
                 FROM source_sync_state WHERE kind='gmail'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (1, 2, "pending".into()));
    }

    #[test]
    fn interrupted_source_work_keeps_its_stage() {
        let (_directory, connection) = database();
        request(&connection, WorkKind::Whatsapp, "test", Duration::zero()).unwrap();
        connection
            .execute(
                "UPDATE source_sync_state SET state='running', step='relationships'
                 WHERE kind='whatsapp'",
                [],
            )
            .unwrap();

        assert_eq!(recover_interrupted(&connection).unwrap(), 1);

        let state: (String, String) = connection
            .query_row(
                "SELECT state, step FROM source_sync_state WHERE kind='whatsapp'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("pending".into(), "relationships".into()));
    }
}
