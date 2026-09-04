use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Row, Table, Wrap};
use ratatui::{Frame, Terminal};

use super::{Status, grouped};
use crate::error::{CrmError, Result};
use crate::progress::ProgressSnapshot;

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

pub(super) fn run(mut collect: impl FnMut() -> Result<Status>) -> Result<()> {
    let mut terminal = start_terminal()?;
    let _restore = RestoreTerminal;
    let mut scroll = 0_u16;
    let mut status = collect()?;

    loop {
        terminal
            .draw(|frame| render(frame, &status, scroll))
            .map_err(terminal_error)?;

        let deadline = Instant::now() + REFRESH_INTERVAL;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !event::poll(remaining).map_err(terminal_error)? {
                break;
            }
            match event::read().map_err(terminal_error)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => break,
                    KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
                    KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                    KeyCode::PageDown => scroll = scroll.saturating_add(10),
                    KeyCode::Home => scroll = 0,
                    _ => {}
                },
                _ => {}
            }
            terminal
                .draw(|frame| render(frame, &status, scroll))
                .map_err(terminal_error)?;
        }
        status = collect()?;
    }
}

fn start_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().map_err(terminal_error)?;
    let mut stdout = io::stdout();
    if let Err(source) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(terminal_error(source));
    }
    Terminal::new(CrosstermBackend::new(stdout)).map_err(|source| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        terminal_error(source)
    })
}

struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn terminal_error(source: io::Error) -> CrmError {
    CrmError::Io {
        path: "terminal".into(),
        source,
    }
}

fn render(frame: &mut Frame, status: &Status, scroll: u16) {
    let area = frame.area();
    let compact = area.height < 28;
    let sections = if compact {
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(5),
        ])
        .split(area)
    } else {
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(7),
        ])
        .split(area)
    };

    render_header(frame, sections[0], status);
    render_summary(frame, sections[1], status);
    render_work(frame, sections[2], status);
    if !compact {
        render_sources(frame, sections[3], status);
        render_events(frame, sections[4], status, scroll);
    }
}

fn render_header(frame: &mut Frame, area: Rect, status: &Status) {
    let state = if status.daemon_running {
        Span::styled("● RUNNING", Style::new().fg(Color::Green).bold())
    } else {
        Span::styled("● STOPPED", Style::new().fg(Color::Red).bold())
    };
    let pid = status
        .daemon_pid
        .map(|pid| format!("  PID {pid}"))
        .unwrap_or_default();
    let title = Line::from(vec![
        Span::styled(" Personal CRM ", Style::new().bold()),
        state,
        Span::raw(pid),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(
            Block::default().borders(Borders::ALL).title_bottom(
                Line::from(" q/Esc/Ctrl-C quit  r refresh  ↑↓ scroll ").right_aligned(),
            ),
        ),
        area,
    );
}

fn render_summary(frame: &mut Frame, area: Rect, status: &Status) {
    let text = vec![
        Line::from(vec![
            metric("CONTACTS", status.total_contacts, Color::Cyan),
            Span::raw("   "),
            metric("INTERACTIONS", status.total_interactions, Color::Magenta),
        ]),
        Line::from(vec![
            metric("PENDING", status.pending_work, Color::Yellow),
            Span::raw("   "),
            metric("RUNNING", status.running_work, Color::Cyan),
            Span::raw("   "),
            metric("FAILED", status.failed_work, Color::Red),
            Span::raw("   "),
            metric("DIRTY", status.dirty_people, Color::Magenta),
            Span::raw("   "),
            metric("REVIEWS", status.pending_reviews, Color::Blue),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Overview "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn metric(label: &'static str, value: i64, color: Color) -> Span<'static> {
    Span::styled(
        format!("{label} {value}"),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn render_work(frame: &mut Frame, area: Rect, status: &Status) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Current activity ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if !status.daemon_running || status.running_activity.is_empty() {
        let message = if status.daemon_running {
            "Waiting for work."
        } else {
            "Daemon is stopped; no live activity."
        };
        frame.render_widget(Paragraph::new(message).dim(), inner);
        return;
    }

    let heights = status
        .running_activity
        .iter()
        .map(|progress| Constraint::Length(3 + visible_focus_len(progress) as u16))
        .collect::<Vec<_>>();
    for (progress, work_area) in status.running_activity.iter().zip(
        Layout::new(Direction::Vertical, heights)
            .split(inner)
            .iter(),
    ) {
        render_work_item(frame, *work_area, progress);
    }
}

fn render_work_item(frame: &mut Frame, area: Rect, progress: &ProgressSnapshot) {
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(visible_focus_len(progress) as u16),
    ])
    .split(area);
    let total = progress.total.max(progress.current);
    let ratio = if total == 0 {
        1.0
    } else {
        progress.current as f64 / total as f64
    };
    let estimate = if progress.total_is_estimate { "~" } else { "" };
    let unit = progress.unit.as_deref().unwrap_or("items");
    let label = format!(
        "{} / {}{} {}  •  stage {} / {}",
        grouped(progress.current),
        estimate,
        grouped(total),
        unit,
        progress.stage_current,
        progress.stage_total
    );
    let title = format!(
        " {}  {} ",
        progress.work_kind.as_deref().unwrap_or("work"),
        progress.message
    );
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::TOP).title(title))
            .gauge_style(Style::new().fg(Color::Cyan))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label)
            .use_unicode(true),
        sections[0],
    );
    if !progress.focus.is_empty() {
        let mut lines = progress
            .focus
            .iter()
            .take(3)
            .map(|item| {
                Line::from(vec![
                    Span::styled("  • ", Style::new().fg(Color::DarkGray)),
                    Span::raw(item),
                ])
            })
            .collect::<Vec<_>>();
        if progress.focus.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("    +{} more in this batch", progress.focus.len() - 3),
                Style::new().fg(Color::DarkGray),
            )));
        }
        frame.render_widget(Paragraph::new(lines), sections[1]);
    }
}

fn visible_focus_len(progress: &ProgressSnapshot) -> usize {
    progress.focus.len().min(3) + usize::from(progress.focus.len() > 3)
}

fn render_sources(frame: &mut Frame, area: Rect, status: &Status) {
    let rows = status.sources.iter().map(|source| {
        Row::new([
            source.id.clone(),
            source.status.clone(),
            source.last_sync_at.as_deref().unwrap_or("-").to_owned(),
            source.error.as_deref().unwrap_or("").to_owned(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(26),
            Constraint::Min(10),
        ],
    )
    .header(Row::new(["SOURCE", "STATE", "LAST SYNC", "ERROR"]).bold())
    .block(Block::default().borders(Borders::ALL).title(" Sources "));
    frame.render_widget(table, area);
}

fn render_events(frame: &mut Frame, area: Rect, status: &Status, scroll: u16) {
    let events: Vec<Line<'_>> = status
        .running_activity
        .iter()
        .flat_map(|progress| progress.events.iter())
        .map(|event| {
            Line::from(vec![
                Span::styled(local_time(&event.at), Style::new().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::raw(&event.message),
            ])
        })
        .collect();
    let content = if events.is_empty() {
        Paragraph::new("No recent activity.").dim()
    } else {
        Paragraph::new(events).scroll((scroll, 0))
    };
    frame.render_widget(
        content.block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Recent activity "),
        ),
        area,
    );
}

fn local_time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| "--:--:--".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_live_dashboard_with_exact_progress() {
        let mut status = super::super::initialized("config.toml".into(), "crm.sqlite3".into(), 10);
        status.daemon_running = true;
        status.total_contacts = 765;
        status.total_interactions = 5_274;
        status.running_activity.push(ProgressSnapshot {
            state: "running".into(),
            message: "Reading WhatsApp conversations".into(),
            stage_current: 2,
            stage_total: 4,
            current: 25,
            total: 100,
            unit: Some("messages".into()),
            focus: vec![
                "Alex · WhatsApp · incoming · Sep 3".into(),
                "Jamie · WhatsApp · outgoing · Sep 3".into(),
                "Morgan · WhatsApp · incoming · Sep 3".into(),
                "Taylor · WhatsApp · outgoing · Sep 3".into(),
            ],
            ..ProgressSnapshot::default()
        });
        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &status, 0)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("Reading WhatsApp conversations"));
        assert!(screen.contains("CONTACTS 765"));
        assert!(screen.contains("INTERACTIONS 5274"));
        assert!(screen.contains("25 / 100 messages"));
        assert!(screen.contains("stage 2 / 4"));
        assert!(screen.contains("Alex · WhatsApp · incoming · Sep 3"));
        assert!(screen.contains("+1 more in this batch"));
    }
}
