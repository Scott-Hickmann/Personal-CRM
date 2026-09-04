use super::super::model::WorkStatus;
use super::*;
use crate::progress::{ProgressEvent, ProgressSnapshot};
use chrono::Utc;
use ratatui::backend::TestBackend;

#[test]
fn renders_pipeline_generations_progress_and_events() {
    let mut status = synthetic_status();
    status.daemon_running = true;
    status.daemon_pid = Some(4217);
    status.running_work = 1;
    status.pending_work = 1;
    status.work.push(running_imessage());
    status.work.push(queued_scoring());

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &status, 0)).unwrap();
    let screen = terminal.backend().to_string();

    assert!(screen.contains("Pipeline map · actual coordinator order"));
    assert!(screen.contains("iMessage"));
    assert!(screen.contains("Reconcile relationships"));
    assert!(screen.contains("319 / 612 conversations"));
    assert!(screen.contains("Generation 14 running · generation 15 queued"));
    assert!(screen.contains("Scoring"));
    assert!(screen.contains("source data changed"));
    assert!(screen.contains("Relationship reconciliation started"));
}

#[test]
fn compact_view_keeps_the_pipeline_visible() {
    let mut status = synthetic_status();
    status.work.push(running_imessage());
    status.work.push(queued_scoring());
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &status, 0)).unwrap();
    let screen = terminal.backend().to_string();

    assert!(screen.contains("SOURCE WORK"));
    assert!(screen.contains("DERIVED / MAINTENANCE WORK"));
    assert!(screen.contains("iMessage"));
    assert!(screen.contains("Scoring"));
}

fn synthetic_status() -> Status {
    Status::initialized("config.toml".into(), "crm.sqlite3".into(), 19)
}

fn running_imessage() -> WorkStatus {
    let now = Utc::now().to_rfc3339();
    WorkStatus {
        kind: "imessage".into(),
        state: "running".into(),
        step: Some("relationships".into()),
        reason: Some("iMessage store changed".into()),
        run_after: now.clone(),
        requested_generation: 15,
        running_generation: Some(14),
        completed_generation: 13,
        attempts: 1,
        changed: true,
        error: None,
        updated_at: now.clone(),
        pending_position: None,
        downstream: vec!["scoring", "suggestions"],
        progress: Some(ProgressSnapshot {
            work_kind: Some("imessage".into()),
            generation: Some(14),
            reason: Some("iMessage store changed".into()),
            state: "running".into(),
            message: "Rebuilding relationship contexts".into(),
            phase_id: Some("relationships".into()),
            phase_label: Some("Reconcile relationships".into()),
            phase_current: 2,
            phase_total: 3,
            stage_current: 1,
            stage_total: 1,
            current: 319,
            total: 612,
            total_is_estimate: false,
            unit: Some("conversations".into()),
            focus: vec!["Family chat".into()],
            started_at: now.clone(),
            updated_at: now.clone(),
            events: vec![ProgressEvent {
                at: now,
                message: "Relationship reconciliation started".into(),
            }],
        }),
    }
}

fn queued_scoring() -> WorkStatus {
    WorkStatus {
        kind: "scoring".into(),
        state: "pending".into(),
        step: None,
        reason: Some("source data changed".into()),
        run_after: "2020-01-01T00:00:00Z".into(),
        requested_generation: 4,
        running_generation: None,
        completed_generation: 3,
        attempts: 0,
        changed: false,
        error: None,
        updated_at: "2020-01-01T00:00:00Z".into(),
        pending_position: Some(1),
        downstream: Vec::new(),
        progress: None,
    }
}
