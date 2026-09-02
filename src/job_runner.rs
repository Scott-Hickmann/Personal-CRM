use std::path::Path;

use crate::error::Result;
use crate::jobs::JobKind;
use crate::progress::ProgressTracker;
use crate::sync::SyncTarget;
use crate::{analysis, commands, contact_commands, photos_commands, review, scoring, sync};

pub(crate) fn run(config_path: &Path, kind: JobKind) -> Result<()> {
    let mut progress = ProgressTracker::disabled();
    run_with_progress(config_path, kind, &mut progress).map(|_| ())
}

pub(crate) fn run_with_progress(
    config_path: &Path,
    kind: JobKind,
    progress: &mut ProgressTracker,
) -> Result<bool> {
    let config = crate::config::Config::load(config_path)?;
    let connection = commands::open_database(config_path)?;
    let changed = match kind {
        JobKind::Contacts => run_sync(SyncTarget::Contacts, &config, &connection, progress)?,
        JobKind::Imessage => run_sync(SyncTarget::Imessage, &config, &connection, progress)?,
        JobKind::Whatsapp => run_sync(SyncTarget::Whatsapp, &config, &connection, progress)?,
        JobKind::AppleCalls => run_sync(SyncTarget::AppleCalls, &config, &connection, progress)?,
        JobKind::WhatsappCalls => {
            run_sync(SyncTarget::WhatsappCalls, &config, &connection, progress)?
        }
        JobKind::Gmail => run_sync(SyncTarget::Gmail, &config, &connection, progress)?,
        JobKind::Analysis => {
            analysis::run(&config, &connection, progress)?;
            true
        }
        JobKind::Scoring => {
            scoring::recalculate_all(&connection, progress)?;
            true
        }
        JobKind::Photos => {
            photos_commands::reconcile_automatic(config_path, progress)?;
            true
        }
        JobKind::GooglePublish => {
            contact_commands::publish_automatic(config_path, progress)?;
            true
        }
        JobKind::Suggestions => {
            review::enqueue_unresolved_candidates_with_progress(&connection, progress)?;
            true
        }
    };
    Ok(changed)
}

fn run_sync(
    target: SyncTarget,
    config: &crate::config::Config,
    connection: &rusqlite::Connection,
    progress: &mut ProgressTracker,
) -> Result<bool> {
    Ok(
        sync::run_with_progress(target, config, connection, progress)?
            .iter()
            .any(|report| report.changed),
    )
}
