use super::*;
use crate::db;

#[test]
fn coalesces_open_jobs_by_kind() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    enqueue(
        &connection,
        JobKind::Contacts,
        "first",
        Duration::seconds(5),
    )
    .unwrap();
    enqueue(&connection, JobKind::Contacts, "second", Duration::zero()).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn requests_a_follow_up_when_enqueued_while_running() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    enqueue(&connection, JobKind::Photos, "first", Duration::zero()).unwrap();
    connection
        .execute("UPDATE jobs SET state='running'", [])
        .unwrap();

    enqueue(
        &connection,
        JobKind::Photos,
        "changed",
        Duration::seconds(5),
    )
    .unwrap();

    let rerun: (i64, Option<String>) = connection
        .query_row("SELECT rerun_requested, rerun_after FROM jobs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(rerun.0, 1);
    assert!(rerun.1.is_some());
}

#[test]
fn exposes_different_workstreams_as_ready_together() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    enqueue(&connection, JobKind::Gmail, "mail", Duration::zero()).unwrap();
    enqueue(&connection, JobKind::Photos, "photos", Duration::zero()).unwrap();
    enqueue(&connection, JobKind::Analysis, "analysis", Duration::zero()).unwrap();

    let kinds: std::collections::HashSet<_> = ready(&connection)
        .unwrap()
        .into_iter()
        .map(|(_, kind)| kind)
        .collect();

    assert_eq!(kinds.len(), 3);
    assert!(kinds.contains(&JobKind::Gmail));
    assert!(kinds.contains(&JobKind::Photos));
    assert!(kinds.contains(&JobKind::Analysis));
}

#[test]
fn recovers_jobs_interrupted_by_a_daemon_restart() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    enqueue(
        &connection,
        JobKind::GooglePublish,
        "test",
        Duration::zero(),
    )
    .unwrap();
    connection
        .execute("UPDATE jobs SET state='running'", [])
        .unwrap();

    assert_eq!(recover_running(&connection).unwrap(), 1);
    let (state, error): (String, Option<String>) = connection
        .query_row("SELECT state, error FROM jobs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(
        error.as_deref(),
        Some("daemon restarted while job was running")
    );
}

#[test]
fn counts_unresolved_failure_kinds_instead_of_historical_rows() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO jobs(kind, state, reason) VALUES
                ('gmail', 'failed', 'first'),
                ('gmail', 'failed', 'second'),
                ('analysis', 'failed', 'third');",
        )
        .unwrap();
    assert_eq!(unresolved_failed_count(&connection).unwrap(), 2);

    connection
        .execute(
            "INSERT INTO jobs(kind, state, reason) VALUES ('gmail', 'complete', 'recovered')",
            [],
        )
        .unwrap();
    assert_eq!(unresolved_failed_count(&connection).unwrap(), 1);
}

#[test]
fn continues_resumable_gmail_backfill_after_each_batch() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind, last_sync_at)
             VALUES ('gmail:test', 'gmail', CURRENT_TIMESTAMP);
             INSERT INTO gmail_sync_scopes(source_id, scope_key, kind, query)
             VALUES ('gmail:test', 'contact:test', 'contact', 'from:test@example.com');",
        )
        .unwrap();

    enqueue_downstream(&connection, JobKind::Gmail, false).unwrap();

    let gmail_jobs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM jobs WHERE kind='gmail' AND state='queued'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(gmail_jobs, 1);
}
