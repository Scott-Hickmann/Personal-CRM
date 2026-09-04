use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::model::Status;

pub(super) fn render(frame: &mut Frame, area: Rect, status: &Status, scroll: u16) {
    let mut events = status
        .work
        .iter()
        .flat_map(|work| {
            work.progress
                .iter()
                .flat_map(|progress| progress.events.iter())
                .map(|event| (event.at.as_str(), work.label(), event.message.as_str()))
        })
        .collect::<Vec<_>>();
    events.sort_by(|left, right| right.0.cmp(left.0));
    let lines = events
        .into_iter()
        .map(|(at, kind, message)| {
            Line::from(vec![
                Span::styled(local_time(at), Style::new().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(format!("{kind:<15}"), Style::new().fg(Color::Cyan)),
                Span::raw(message.to_owned()),
            ])
        })
        .collect::<Vec<_>>();
    let content = if lines.is_empty() {
        Paragraph::new("No recorded activity.").dim()
    } else {
        Paragraph::new(lines).scroll((scroll, 0))
    };
    frame.render_widget(
        content.block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Recent phase transitions · newest first "),
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
