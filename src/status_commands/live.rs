mod details;
mod events;
mod pipeline;

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::model::Status;
use crate::error::{CrmError, Result};

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
    let compact = area.height < 32 || (area.width < 100 && area.height < 42);
    let sections = if compact {
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(10),
        ])
        .split(area)
    } else {
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(8),
        ])
        .split(area)
    };
    render_header(frame, sections[0], status);
    render_summary(frame, sections[1], status);
    if compact {
        pipeline::render(frame, sections[2], status);
    } else if area.width >= 100 {
        let middle = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(sections[2]);
        pipeline::render(frame, middle[0], status);
        details::render(frame, middle[1], status);
        events::render(frame, sections[3], status, scroll);
    } else {
        let middle =
            Layout::vertical([Constraint::Min(10), Constraint::Length(10)]).split(sections[2]);
        pipeline::render(frame, middle[0], status);
        details::render(frame, middle[1], status);
        events::render(frame, sections[3], status, scroll);
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
    let heartbeat = status
        .daemon_heartbeat_at
        .as_deref()
        .and_then(age)
        .map(|age| format!("  heartbeat {} ago", human_duration(age)))
        .unwrap_or_default();
    let title = Line::from(vec![
        Span::styled(" Personal CRM ", Style::new().bold()),
        state,
        Span::raw(pid),
        Span::styled("  one writer", Style::new().fg(Color::DarkGray)),
        Span::styled(heartbeat, Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(
            Block::default().borders(Borders::ALL).title_bottom(
                Line::from(" q/Esc/Ctrl-C quit  r refresh  ↑↓ events ").right_aligned(),
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
            metric("QUEUED", status.pending_work, Color::Yellow),
            Span::raw("   "),
            metric("RUNNING", status.running_work, Color::Cyan),
            Span::raw("   "),
            metric("FAILED", status.failed_work, Color::Red),
            Span::raw("   "),
            metric("DIRTY PEOPLE", status.dirty_people, Color::Magenta),
            Span::raw("   "),
            metric("DIRTY CHATS", status.dirty_conversations, Color::Blue),
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

pub(super) fn age(value: &str) -> Option<Duration> {
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map(|value| value.and_utc())
        })
        .ok()?;
    (Utc::now() - timestamp).to_std().ok()
}

pub(super) fn human_duration(value: Duration) -> String {
    if value.as_secs() < 60 {
        format!("{}s", value.as_secs())
    } else if value.as_secs() < 3_600 {
        format!("{}m", value.as_secs() / 60)
    } else {
        format!("{}h", value.as_secs() / 3_600)
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;
