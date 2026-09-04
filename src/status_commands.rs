use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::cli::StatusArgs;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
mod live;

#[derive(Serialize)]
pub(crate) struct Status {
    total_contacts: i64,
    total_analyzable_interactions: i64,
    analyzed_interactions: i64,
}

pub(crate) fn initialized(
    _config_path: PathBuf,
    _database_path: PathBuf,
    _schema_version: i64,
) -> Status {
    Status {
        total_contacts: 0,
        total_analyzable_interactions: 0,
        analyzed_interactions: 0,
    }
}

pub(crate) fn run(format: Format, config_path: PathBuf, args: StatusArgs) -> Result<()> {
    let connection = crate::commands::open_database(&config_path)?;
    if args.live {
        live(format, &config_path, &connection)
    } else {
        let status = collect(&config_path, &connection)?;
        let table = summary(&status);
        output::emit(format, "status", &status, table)
    }
}

fn live(format: Format, config_path: &Path, connection: &Connection) -> Result<()> {
    if !matches!(format, Format::Table) {
        return Err(CrmError::InvalidConfig(
            "`crm status --live` only supports table output".into(),
        ));
    }
    if !std::io::stdout().is_terminal() {
        return Err(CrmError::InvalidConfig(
            "`crm status --live` requires an interactive terminal".into(),
        ));
    }
    live::run(|| collect(config_path, connection))
}

fn collect(_config_path: &Path, connection: &Connection) -> Result<Status> {
    let interaction_counts = crate::analysis::counts(connection)?;
    Ok(Status {
        total_contacts: connection.query_row(
            "SELECT COUNT(*) FROM people WHERE lifecycle_state='active'",
            [],
            |row| row.get(0),
        )?,
        total_analyzable_interactions: interaction_counts.total,
        analyzed_interactions: interaction_counts.analyzed,
    })
}

fn summary(status: &Status) -> String {
    format!(
        "contacts                 {}\nanalyzable interactions  {}\nanalyzed interactions    {}",
        grouped(status.total_contacts),
        grouped(status.total_analyzable_interactions),
        grouped(status.analyzed_interactions),
    )
}

pub(super) fn grouped(value: i64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_large_counts() {
        assert_eq!(grouped(20_280), "20,280");
    }

    #[test]
    fn summarizes_only_requested_totals() {
        let status = Status {
            total_contacts: 765,
            total_analyzable_interactions: 5_274,
            analyzed_interactions: 4_359,
        };

        assert_eq!(
            summary(&status),
            "contacts                 765\nanalyzable interactions  5,274\nanalyzed interactions    4,359"
        );
    }
}
