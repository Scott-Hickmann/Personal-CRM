use std::path::Path;

use crate::error::Result;
use crate::jobs::JobKind;
use crate::progress::ProgressTracker;
use crate::sync::SyncTarget;
use crate::{analysis, commands, contact_commands, photos_commands, review, scoring, sync};

pub(crate) fn run(config_path: &Path, kind: JobKind) -> Result<()> {
    let mut progress = ProgressTracker::disabled();
    run_with_progress(config_path, kind, &mut progress)
}

pub(crate) fn run_with_progress(
    config_path: &Path,
    kind: JobKind,
    progress: &mut ProgressTracker,
) -> Result<()> {
    let config = crate::config::Config::load(config_path)?;
    let connection = commands::open_database(config_path)?;
    match kind {
        JobKind::Contacts => {
            sync::run_with_progress(SyncTarget::Contacts, &config, &connection, progress)?;
        }
        JobKind::Communications => {
            sync::run_with_progress(SyncTarget::Imessage, &config, &connection, progress)?;
            sync::run_with_progress(SyncTarget::Whatsapp, &config, &connection, progress)?;
            sync::run_with_progress(SyncTarget::Calls, &config, &connection, progress)?;
        }
        JobKind::Gmail => {
            sync::run_with_progress(SyncTarget::Gmail, &config, &connection, progress)?;
        }
        JobKind::Analysis => {
            analysis::run(&config, &connection, 100, progress)?;
        }
        JobKind::Scoring => {
            scoring::recalculate_all(&connection, progress)?;
        }
        JobKind::Photos => {
            photos_commands::reconcile_automatic(config_path, progress)?;
        }
        JobKind::GooglePublish => {
            contact_commands::publish_automatic(config_path, progress)?;
        }
        JobKind::Suggestions => {
            review::enqueue_unresolved_candidates_with_progress(&connection, progress)?;
        }
    }
    Ok(())
}
