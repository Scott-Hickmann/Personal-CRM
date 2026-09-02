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
            progress.phase("Reading iCloud Contacts");
            sync::run_with_progress(SyncTarget::Contacts, &config, &connection, progress)?;
        }
        JobKind::Communications => {
            progress.phase("Reading iMessage conversations");
            sync::run_with_progress(SyncTarget::Imessage, &config, &connection, progress)?;
            progress.phase("Reading WhatsApp conversations");
            sync::run_with_progress(SyncTarget::Whatsapp, &config, &connection, progress)?;
            progress.phase("Reading call history");
            sync::run_with_progress(SyncTarget::Calls, &config, &connection, progress)?;
        }
        JobKind::Gmail => {
            progress.phase("Connecting to Gmail");
            sync::run_with_progress(SyncTarget::Gmail, &config, &connection, progress)?;
        }
        JobKind::Analysis => {
            progress.phase("Analyzing recent interactions");
            analysis::run(&config, &connection, 100)?;
        }
        JobKind::Scoring => {
            progress.phase("Recalculating relationship scores");
            scoring::recalculate_all(&connection)?;
        }
        JobKind::Photos => {
            progress.phase("Reconciling Photos people");
            photos_commands::reconcile_automatic(config_path)?;
        }
        JobKind::GooglePublish => {
            progress.phase("Publishing iCloud contacts to Google Contacts");
            contact_commands::publish_automatic(config_path)?;
        }
        JobKind::Suggestions => {
            progress.phase("Finding unresolved contact suggestions");
            review::enqueue_unresolved_candidates(&connection)?;
        }
    }
    Ok(())
}
