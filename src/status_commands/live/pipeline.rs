use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::model::{Status, WorkStatus};

pub(super) fn render(frame: &mut Frame, area: Rect, status: &Status) {
    let sources = status.work.iter().filter(|work| work.is_source());
    let maintenance = status.work.iter().filter(|work| !work.is_source());
    let mut lines = vec![Line::from(Span::styled(
        "SOURCE WORK",
        Style::new().fg(Color::DarkGray).bold(),
    ))];
    lines.extend(sources.map(work_line));
    lines.push(Line::from(vec![
        Span::styled(
            "     └─ on change ───────────▶ ",
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled("derived work", Style::new().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(Span::styled(
        "DERIVED / MAINTENANCE WORK",
        Style::new().fg(Color::DarkGray).bold(),
    )));
    lines.extend(maintenance.map(work_line));
    lines.push(Line::from(Span::styled(
        "any changed source ─▶ Scoring + Suggestions",
        Style::new().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "Contacts changed ─▶ Gmail + Google publish · Gmail backfill ↻ Gmail",
        Style::new().fg(Color::DarkGray),
    )));
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Pipeline map · actual coordinator order "),
        ),
        area,
    );
}

fn work_line(work: &WorkStatus) -> Line<'static> {
    let (marker, color) = marker(work);
    let rerun = if work.rerun_queued() {
        "  ↻ rerun queued"
    } else {
        ""
    };
    Line::from(vec![
        Span::styled(format!("{marker:>3} "), Style::new().fg(color).bold()),
        Span::styled(format!("{:<17}", work.label()), Style::new().bold()),
        Span::styled(description(work), Style::new().fg(color)),
        Span::styled(rerun, Style::new().fg(Color::Yellow).bold()),
    ])
}

fn marker(work: &WorkStatus) -> (String, Color) {
    match work.state.as_str() {
        "running" => ("●".into(), Color::Cyan),
        "failed" => ("!".into(), Color::Red),
        "pending" if work.attempts > 0 => ("◷".into(), Color::Yellow),
        "pending" if work.ready() => (
            work.pending_position.unwrap_or_default().to_string(),
            Color::Yellow,
        ),
        "pending" => ("◷".into(), Color::Yellow),
        _ => ("✓".into(), Color::Green),
    }
}

fn description(work: &WorkStatus) -> String {
    match work.state.as_str() {
        "running" => work
            .progress
            .as_ref()
            .map(progress_description)
            .unwrap_or_else(|| durable_phase(work).into()),
        "failed" => format!(
            "failed · {}",
            work.error.as_deref().unwrap_or("unknown error")
        ),
        "pending" if work.attempts > 0 => format!(
            "retry {} · {}",
            short_time(&work.run_after),
            work.reason.as_deref().unwrap_or("previous attempt failed")
        ),
        "pending" if work.ready() => {
            format!("queued · {}", work.reason.as_deref().unwrap_or("requested"))
        }
        "pending" => format!(
            "scheduled {} · {}",
            short_time(&work.run_after),
            work.reason.as_deref().unwrap_or("requested")
        ),
        _ => "up to date".into(),
    }
}

fn progress_description(progress: &crate::progress::ProgressSnapshot) -> String {
    let phase = progress
        .phase_label
        .as_deref()
        .unwrap_or(progress.message.as_str());
    if progress.total > 1 || progress.current > 0 {
        let estimate = if progress.total_is_estimate { "~" } else { "" };
        format!(
            "{} · {}/{}{} {}",
            phase,
            progress.current,
            estimate,
            progress.total.max(progress.current),
            progress.unit.as_deref().unwrap_or("items")
        )
    } else {
        phase.into()
    }
}

fn durable_phase(work: &WorkStatus) -> &'static str {
    match work.step.as_deref() {
        Some("relationships") => "Reconcile relationships",
        Some("dirty_people") => "Mark affected people",
        _ => "Import source data",
    }
}

fn short_time(value: &str) -> String {
    value
        .split('T')
        .nth(1)
        .or_else(|| value.split(' ').nth(1))
        .unwrap_or(value)
        .trim_end_matches('Z')
        .split('.')
        .next()
        .unwrap_or(value)
        .into()
}
