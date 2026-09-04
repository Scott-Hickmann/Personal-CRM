use super::*;
use crate::coordinator::{self, WorkKind};
use chrono::Duration;

#[test]
fn recognizes_ready_rfc3339_and_sqlite_timestamps() {
    assert!(ready_at("2020-01-01T00:00:00Z"));
    assert!(ready_at("2020-01-01 00:00:00"));
    assert!(!ready_at("2999-01-01T00:00:00Z"));
}

#[test]
fn projects_every_work_row_in_actual_pending_order() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    coordinator::request(
        &connection,
        WorkKind::Gmail,
        "history poll",
        Duration::zero(),
    )
    .unwrap();
    coordinator::request(
        &connection,
        WorkKind::Whatsapp,
        "store changed",
        Duration::zero(),
    )
    .unwrap();

    let status = collect(&config_path, &connection).unwrap();

    assert_eq!(status.work.len(), 10);
    let whatsapp = status
        .work
        .iter()
        .find(|work| work.kind == "whatsapp")
        .unwrap();
    let gmail = status
        .work
        .iter()
        .find(|work| work.kind == "gmail")
        .unwrap();
    assert_eq!(whatsapp.pending_position, Some(1));
    assert_eq!(gmail.pending_position, Some(2));
    assert_eq!(whatsapp.reason.as_deref(), Some("store changed"));
}
