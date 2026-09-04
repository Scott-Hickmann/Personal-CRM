use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

use super::super::grouped;
use super::super::model::{Status, WorkStatus};
use super::{age, human_duration};

const STALE_AFTER: Duration = Duration::from_secs(3);

pub(super) fn render(frame: &mut Frame, area: Rect, status: &Status) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Current path ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(work) = status.active_or_next() else {
        frame.render_widget(
            Paragraph::new("✓ All work is up to date.").fg(Color::Green),
            inner,
        );
        return;
    };
    let progress = work
        .progress
        .as_ref()
        .filter(|progress| work.state == "running" && progress.state == "running");
    let areas = if progress.is_some() {
        Layout::vertical([Constraint::Min(6), Constraint::Length(3)]).split(inner)
    } else {
        Layout::vertical([Constraint::Min(1), Constraint::Length(0)]).split(inner)
    };
    frame.render_widget(Paragraph::new(detail_lines(work, progress)), areas[0]);
    if let Some(progress) = progress {
        render_gauge(frame, areas[1], progress);
    }
}

fn detail_lines(
    work: &WorkStatus,
    progress: Option<&crate::progress::ProgressSnapshot>,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        work.label(),
        Style::new().bold().fg(Color::Cyan),
    ))];
    if work.is_source() {
        let active = active_source_phase(work, progress);
        for (index, label) in [
            "Import source data",
            "Reconcile relationships",
            "Mark affected people",
        ]
        .iter()
        .enumerate()
        {
            let phase = index + 1;
            let (symbol, color) = if phase < active {
                ("✓", Color::Green)
            } else if phase == active && work.state == "failed" {
                ("!", Color::Red)
            } else if phase == active && work.state == "running" {
                ("●", Color::Cyan)
            } else {
                ("○", Color::DarkGray)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{symbol} "), Style::new().fg(color).bold()),
                Span::raw(format!("{phase}. {label}")),
            ]));
        }
    } else if let Some(progress) = progress {
        lines.push(Line::from(format!(
            "● {} · operation {}/{}",
            progress.message, progress.stage_current, progress.stage_total
        )));
    }
    lines.push(Line::from(generation_line(work)));
    if let Some(reason) = work.reason.as_deref() {
        lines.push(Line::from(vec![
            Span::styled("Reason: ", Style::new().fg(Color::DarkGray)),
            Span::raw(reason.to_owned()),
        ]));
    }
    if let Some(progress) = progress {
        lines.push(timing_line(progress));
        for item in progress.focus.iter().take(2) {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::new().fg(Color::DarkGray)),
                Span::raw(item.clone()),
            ]));
        }
    } else if work.state == "failed" {
        lines.push(Line::from(Span::styled(
            work.error.clone().unwrap_or_else(|| "Unknown error".into()),
            Style::new().fg(Color::Red),
        )));
    }
    lines
}

fn active_source_phase(
    work: &WorkStatus,
    progress: Option<&crate::progress::ProgressSnapshot>,
) -> usize {
    match progress
        .and_then(|progress| progress.phase_id.as_deref())
        .or(work.step.as_deref())
    {
        Some("relationships") => 2,
        Some("dirty_people") => 3,
        _ => 1,
    }
}

fn generation_line(work: &WorkStatus) -> String {
    if let Some(running) = work.running_generation {
        if work.rerun_queued() {
            format!(
                "Generation {running} running · generation {} queued",
                work.requested_generation
            )
        } else {
            format!("Generation {running} running")
        }
    } else {
        format!(
            "Generation {}/{} completed",
            work.completed_generation, work.requested_generation
        )
    }
}

fn timing_line(progress: &crate::progress::ProgressSnapshot) -> Line<'static> {
    let elapsed = age(&progress.started_at)
        .map(human_duration)
        .unwrap_or_else(|| "?".into());
    let update_age = age(&progress.updated_at);
    let update = update_age
        .map(human_duration)
        .unwrap_or_else(|| "unavailable".into());
    let stale = update_age.is_none_or(|value| value > STALE_AFTER);
    Line::from(vec![
        Span::raw(format!("Started {elapsed} ago · ")),
        Span::styled(
            if stale {
                format!("detail stale ({update} ago)")
            } else {
                format!("updated {update} ago")
            },
            Style::new().fg(if stale { Color::Yellow } else { Color::Green }),
        ),
    ])
}

fn render_gauge(frame: &mut Frame, area: Rect, progress: &crate::progress::ProgressSnapshot) {
    let total = progress.total.max(progress.current);
    let ratio = if total == 0 {
        0.0
    } else {
        progress.current as f64 / total as f64
    };
    let estimate = if progress.total_is_estimate { "~" } else { "" };
    let label = format!(
        "{} / {}{} {} · operation {}/{}",
        grouped(progress.current),
        estimate,
        grouped(total),
        progress.unit.as_deref().unwrap_or("items"),
        progress.stage_current,
        progress.stage_total
    );
    frame.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(format!(" {} ", progress.message)),
            )
            .gauge_style(Style::new().fg(Color::Cyan))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label)
            .use_unicode(true),
        area,
    );
}
