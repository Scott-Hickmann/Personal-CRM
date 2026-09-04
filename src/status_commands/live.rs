use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use super::{Status, grouped};
use crate::error::{CrmError, Result};

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

pub(super) fn run(mut collect: impl FnMut() -> Result<Status>) -> Result<()> {
    let mut terminal = start_terminal()?;
    let _restore = RestoreTerminal;
    let mut status = collect()?;

    loop {
        terminal
            .draw(|frame| render(frame, &status))
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
                    _ => {}
                },
                _ => {}
            }
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

fn render(frame: &mut Frame, status: &Status) {
    let sections =
        Layout::vertical([Constraint::Length(3), Constraint::Length(5)]).split(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Personal CRM ",
            Style::new().add_modifier(Modifier::BOLD),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title_bottom(Line::from(" q/Esc/Ctrl-C quit  r refresh ").right_aligned()),
        ),
        sections[0],
    );
    render_summary(frame, sections[1], status);
}

fn render_summary(frame: &mut Frame, area: Rect, status: &Status) {
    let rows = vec![
        metric("CONTACTS", status.total_contacts, Color::Cyan),
        metric(
            "ANALYZABLE INTERACTIONS",
            status.total_analyzable_interactions,
            Color::Magenta,
        ),
        metric(
            "ANALYZED INTERACTIONS",
            status.analyzed_interactions,
            Color::Green,
        ),
    ];
    frame.render_widget(
        Paragraph::new(rows).block(Block::default().borders(Borders::ALL).title(" Totals ")),
        area,
    );
}

fn metric(label: &'static str, value: i64, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("{label:<26} {}", grouped(value)),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_the_three_requested_totals() {
        let status = Status {
            total_contacts: 765,
            total_analyzable_interactions: 5_274,
            analyzed_interactions: 4_359,
        };
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &status)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("CONTACTS                   765"));
        assert!(screen.contains("ANALYZABLE INTERACTIONS    5,274"));
        assert!(screen.contains("ANALYZED INTERACTIONS      4,359"));
    }
}
