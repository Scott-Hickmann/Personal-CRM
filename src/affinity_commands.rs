use std::path::PathBuf;

use serde::Serialize;

use crate::affinity_calibration;
use crate::cli::{AffinityClearArgs, AffinityCommand, AffinityRateArgs};
use crate::error::Result;
use crate::output::{self, Format};
use crate::progress::ProgressTracker;
use crate::{commands, repository, scoring};

#[derive(Serialize)]
struct RatingResult {
    person_id: String,
    rating: Option<u8>,
    dry_run: bool,
}

pub fn run(format: Format, config_path: PathBuf, command: AffinityCommand) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    match command {
        AffinityCommand::Rate(args) => rate(format, &connection, args),
        AffinityCommand::Clear(args) => clear(format, &connection, args),
    }
}

fn rate(format: Format, connection: &rusqlite::Connection, args: AffinityRateArgs) -> Result<()> {
    let person_id = repository::resolve_person_id(connection, &args.person)?;
    if !args.dry_run {
        affinity_calibration::set_rating(connection, &person_id, args.rating)?;
        scoring::recalculate_all(connection, &mut ProgressTracker::disabled())?;
    }
    let result = RatingResult {
        person_id,
        rating: Some(args.rating),
        dry_run: args.dry_run,
    };
    output::emit(
        format,
        "affinity.rating",
        &result,
        format!("closeness rating set to {}/7", args.rating),
    )
}

fn clear(format: Format, connection: &rusqlite::Connection, args: AffinityClearArgs) -> Result<()> {
    let person_id = repository::resolve_person_id(connection, &args.person)?;
    if !args.dry_run {
        affinity_calibration::clear_rating(connection, &person_id)?;
        scoring::recalculate_all(connection, &mut ProgressTracker::disabled())?;
    }
    let result = RatingResult {
        person_id,
        rating: None,
        dry_run: args.dry_run,
    };
    output::emit(
        format,
        "affinity.rating",
        &result,
        "closeness rating cleared".into(),
    )
}
