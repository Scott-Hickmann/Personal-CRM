use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::source::ReadOnlySource;

const AUDIT_INTERVAL: &str = "-1 day";
const CURSOR_OVERLAP: i64 = 1_000;

pub(super) struct IncrementalSource {
    source: ReadOnlySource,
    pub(super) fingerprint: String,
    pub(super) run_at: String,
    pub(super) cursor: i64,
    pub(super) audit: bool,
}

impl IncrementalSource {
    pub(super) fn connection(&self) -> &Connection {
        self.source.connection()
    }

    pub(super) fn require_columns(&self, table: &str, columns: &[&str]) -> Result<()> {
        self.source.require_columns(table, columns)
    }
}

pub(super) fn open_incremental_source(
    crm: &Connection,
    id: &str,
    kind: &str,
    path: &Path,
    table: &str,
    columns: &[&str],
) -> Result<IncrementalSource> {
    let source = ReadOnlySource::open(path)?;
    source.require_columns(table, columns)?;
    let fingerprint = source.schema_fingerprint()?;
    let run_at = Utc::now().to_rfc3339();
    let previous: Option<(Option<String>, Option<String>, i64)> = crm
        .query_row(
            "SELECT cursor, schema_fingerprint,
                    CASE WHEN last_reconcile_at IS NULL
                          OR datetime(last_reconcile_at) <= datetime('now', ?2)
                         THEN 1 ELSE 0 END
             FROM sources WHERE id=?1",
            params![id, AUDIT_INTERVAL],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let schema_changed = previous
        .as_ref()
        .and_then(|(_, value, _)| value.as_deref())
        .is_some_and(|value| value != fingerprint);
    let cursor = if schema_changed {
        0
    } else {
        previous
            .as_ref()
            .and_then(|(value, _, _)| value.as_deref())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    let audit = cursor == 0
        || schema_changed
        || previous.is_none()
        || previous.is_some_and(|item| item.2 != 0);
    crm.execute(
        "INSERT INTO sources(id, kind, schema_fingerprint, status) VALUES (?1, ?2, ?3, 'syncing')
         ON CONFLICT(id) DO UPDATE SET schema_fingerprint=excluded.schema_fingerprint,
           status='syncing', error=NULL",
        params![id, kind, fingerprint],
    )?;
    Ok(IncrementalSource {
        source,
        fingerprint,
        run_at,
        cursor,
        audit,
    })
}

pub(super) fn incremental_floor(source: &IncrementalSource) -> i64 {
    if source.audit {
        0
    } else {
        source.cursor.saturating_sub(CURSOR_OVERLAP)
    }
}

pub(super) fn finish_incremental_source(
    crm: &Connection,
    id: &str,
    source: &IncrementalSource,
    cursor: i64,
) -> Result<usize> {
    let deleted_ids = if source.audit {
        missing_interactions(crm, id, &source.run_at)?
    } else {
        Vec::new()
    };
    let cursor = if source.audit {
        cursor
    } else {
        cursor.max(source.cursor)
    };
    crm.execute(
        "UPDATE sources SET status='ok', cursor=?2, last_sync_at=CURRENT_TIMESTAMP,
         last_reconcile_at=CASE WHEN ?3 THEN CURRENT_TIMESTAMP ELSE last_reconcile_at END
         WHERE id=?1",
        params![id, cursor.to_string(), source.audit],
    )?;
    Ok(deleted_ids.len())
}

fn missing_interactions(crm: &Connection, id: &str, run_at: &str) -> Result<Vec<String>> {
    let mut statement = crm.prepare(
        "SELECT native_id FROM interactions
         WHERE source_id=?1 AND deleted_at IS NULL
           AND (last_seen_at IS NULL OR last_seen_at < ?2)",
    )?;
    let deleted_ids: Vec<String> = statement
        .query_map(params![id, run_at], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for native_id in &deleted_ids {
        crm.execute(
            "INSERT OR IGNORE INTO tombstones(source_id, native_id) VALUES (?1, ?2)",
            params![id, native_id],
        )?;
    }
    crm.execute(
        "UPDATE interactions SET body=NULL, subject=NULL, deleted_at=CURRENT_TIMESTAMP
         WHERE source_id=?1 AND deleted_at IS NULL
           AND (last_seen_at IS NULL OR last_seen_at < ?2)",
        params![id, run_at],
    )?;
    Ok(deleted_ids)
}

pub(super) fn delete_interactions(
    crm: &Connection,
    source_id: &str,
    native_ids: impl IntoIterator<Item = String>,
) -> Result<usize> {
    let mut deleted = 0;
    for native_id in native_ids {
        crm.execute(
            "INSERT OR IGNORE INTO tombstones(source_id, native_id) VALUES (?1, ?2)",
            params![source_id, native_id],
        )?;
        deleted += crm.execute(
            "UPDATE interactions SET body=NULL, subject=NULL, deleted_at=CURRENT_TIMESTAMP
             WHERE source_id=?1 AND native_id=?2 AND deleted_at IS NULL",
            params![source_id, native_id],
        )?;
    }
    Ok(deleted)
}
