use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::cli::StatusArgs;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};

mod catalog;
mod live;
mod model;

use model::Status;

pub(crate) fn initialized(
    config_path: PathBuf,
    database_path: PathBuf,
    schema_version: i64,
) -> Status {
    Status::initialized(config_path, database_path, schema_version)
}

pub(crate) fn run(format: Format, config_path: PathBuf, args: StatusArgs) -> Result<()> {
    let connection = crate::commands::open_database(&config_path)?;
    if args.live {
        live(format, &config_path, &connection)
    } else {
        let mut status = model::collect(&config_path, &connection)?;
        for work in &mut status.work {
            if let Some(progress) = &mut work.progress {
                progress.focus.clear();
            }
        }
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
    live::run(|| model::collect(config_path, connection))
}

fn summary(status: &Status) -> String {
    let mut output = format!(
        "daemon                    {}{}\ndaemon heartbeat          {}\nschema version             {}\nsources                    {}\ncontacts                   {}\ninteractions               {}\nwork                       {} pending, {} running, {} failed\ndirty conversations        {}\ndirty people               {}\nreview                     {} pending",
        if status.daemon_running {
            "running"
        } else {
            "stopped"
        },
        status
            .daemon_pid
            .map(|pid| format!(" (PID {pid})"))
            .unwrap_or_default(),
        status.daemon_heartbeat_at.as_deref().unwrap_or("-"),
        status.schema_version,
        status.source_count,
        grouped(status.total_contacts as u64),
        grouped(status.total_interactions as u64),
        status.pending_work,
        status.running_work,
        status.failed_work,
        status.dirty_conversations,
        status.dirty_people,
        status.pending_reviews
    );
    let active_work = status
        .work
        .iter()
        .filter(|work| work.state != "idle" || work.error.is_some())
        .collect::<Vec<_>>();
    if !active_work.is_empty() {
        output.push_str("\n\nPersisted work\n");
        for work in active_work {
            output.push_str(&format!(
                "{:<18} {:<8} {:<14} generation {}{}  {}{}\n",
                work.kind,
                work.state,
                work.step.as_deref().unwrap_or("-"),
                work.requested_generation,
                work.running_generation
                    .map(|generation| format!(" (running {generation})"))
                    .unwrap_or_else(|| format!(" (completed {})", work.completed_generation)),
                work.reason.as_deref().unwrap_or("no reason recorded"),
                work.error
                    .as_deref()
                    .map(|error| format!("  error {error}"))
                    .unwrap_or_default(),
            ));
        }
        output.pop();
    }
    if !status.sources.is_empty() {
        output.push_str("\n\nSource status\n");
        for source in &status.sources {
            output.push_str(&format!(
                "{:<18} {:<8} sync {}  audit {}  cursor {}{}\n",
                source.id,
                source.status,
                source.last_sync_at.as_deref().unwrap_or("-"),
                source.last_reconcile_at.as_deref().unwrap_or("-"),
                source.cursor.as_deref().unwrap_or("-"),
                source
                    .error
                    .as_deref()
                    .map(|error| format!("  error {error}"))
                    .unwrap_or_default(),
            ));
        }
        output.pop();
    }
    output
}

pub(super) fn grouped(value: u64) -> String {
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
}
