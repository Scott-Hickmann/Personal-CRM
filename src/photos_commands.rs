use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{PhotosCommand, PhotosLibraryArgs};
use crate::commands;
use crate::error::Result;
use crate::output::{self, Format};
use crate::photo_links;
use crate::photos_library::{self, NamedPhotosPerson, PhotosCatalog};
use crate::photos_prompt;
use crate::progress::ProgressTracker;

#[derive(Serialize)]
struct ReconcileResult {
    linked: usize,
    waiting_for_photos: usize,
    stale_marked: usize,
    stopped_early: bool,
}

pub(crate) fn run(format: Format, config_path: PathBuf, command: PhotosCommand) -> Result<()> {
    match command {
        PhotosCommand::Status => status(format, config_path),
        PhotosCommand::Review(args) => crate::photos_review::run(format, config_path, args),
        PhotosCommand::Reconcile(args) => reconcile(format, config_path, args),
    }
}

fn status(format: Format, config_path: PathBuf) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    let status = photo_links::status(&connection)?;
    let table = format!(
        "total people       {}\npending            {}\ndeferred           {}\nasset linked       {}\nPhotos person      {}\nnot applicable     {}\nstale              {}",
        status.total_people,
        status.pending,
        status.deferred,
        status.asset_linked,
        status.person_linked,
        status.not_applicable,
        status.stale,
    );
    output::emit(format, "photos.status", &status, table)
}

fn reconcile(format: Format, config_path: PathBuf, args: PhotosLibraryArgs) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    let library = photos_library::discover_library(args.library)?;
    let catalog = PhotosCatalog::open(&library)?;
    let current_people = catalog.named_people()?;
    let current_by_id = current_people
        .iter()
        .map(|person| (person.person_uuid.as_str(), person))
        .collect::<HashMap<_, _>>();
    let people = photo_links::review_people(&connection, None)?;
    let mut result = ReconcileResult {
        linked: 0,
        waiting_for_photos: 0,
        stale_marked: 0,
        stopped_early: false,
    };

    for person in people {
        let Some(link) = &person.link else { continue };
        if link.state == "person_linked" {
            let Some(uuid) = link.photos_person_uuid.as_deref() else {
                continue;
            };
            if let Some(current) = current_by_id.get(uuid) {
                if link.photos_name_snapshot.as_deref() != Some(current.name.as_str()) {
                    photo_links::link_photos_person(
                        &connection,
                        &person.person_id,
                        &current.person_uuid,
                        &current.name,
                        current.key_asset_id.as_deref(),
                    )?;
                }
            } else {
                photo_links::set_review_state(&connection, &person.person_id, "stale")?;
                result.stale_marked += 1;
            }
            continue;
        }
        if !matches!(link.state.as_str(), "asset_linked" | "stale") {
            continue;
        }
        let Some(asset_id) = link.photos_asset_id.as_deref() else {
            result.waiting_for_photos += 1;
            continue;
        };
        let candidates = catalog.named_people_for_asset(asset_id)?;
        if candidates.is_empty() {
            result.waiting_for_photos += 1;
            continue;
        }
        eprintln!(
            "\n{} — Photos found named faces on the linked asset:",
            person.display_name
        );
        let selected = select_candidate(&candidates)?;
        let Some(selected) = selected else {
            result.stopped_early = true;
            break;
        };
        photo_links::link_photos_person(
            &connection,
            &person.person_id,
            &selected.person_uuid,
            &selected.name,
            selected.key_asset_id.as_deref(),
        )?;
        result.linked += 1;
    }

    let table = format!(
        "linked {}\nwaiting for Photos {}\nstale links marked {}{}",
        result.linked,
        result.waiting_for_photos,
        result.stale_marked,
        if result.stopped_early {
            "\nstopped early"
        } else {
            ""
        },
    );
    output::emit(format, "photos.reconcile", &result, table)
}

pub(crate) fn reconcile_automatic(
    config_path: &std::path::Path,
    progress: &mut ProgressTracker,
) -> Result<()> {
    progress.stage("Loading named Photos people", 1, 2, 1, false, "query");
    let connection = commands::open_database(config_path)?;
    let library = photos_library::discover_library(None)?;
    let catalog = PhotosCatalog::open(&library)?;
    let current_people = catalog.named_people()?;
    let current_by_id = current_people
        .iter()
        .map(|person| (person.person_uuid.as_str(), person))
        .collect::<HashMap<_, _>>();
    progress.finish_stage("Loaded named Photos people", 1, 1, false, "query");
    let people = photo_links::review_people(&connection, None)?;
    let linked: Vec<_> = people
        .iter()
        .filter(|person| {
            person
                .link
                .as_ref()
                .is_some_and(|link| link.state == "person_linked")
        })
        .collect();
    let total = linked.len() as u64;
    progress.stage("Reconciling Photos people", 2, 2, total, false, "people");
    for (index, person) in linked.into_iter().enumerate() {
        let link = person.link.as_ref().unwrap();
        let Some(uuid) = link.photos_person_uuid.as_deref() else {
            progress.progress(
                "Reconciling Photos people",
                (index + 1) as u64,
                total,
                false,
                "people",
            );
            continue;
        };
        if let Some(current) = current_by_id.get(uuid) {
            if link.photos_name_snapshot.as_deref() != Some(current.name.as_str()) {
                photo_links::link_photos_person(
                    &connection,
                    &person.person_id,
                    &current.person_uuid,
                    &current.name,
                    current.key_asset_id.as_deref(),
                )?;
            }
        } else {
            photo_links::set_review_state(&connection, &person.person_id, "stale")?;
        }
        progress.progress(
            "Reconciling Photos people",
            (index + 1) as u64,
            total,
            false,
            "people",
        );
    }
    progress.finish_stage("Reconciled Photos people", total, total, false, "people");
    Ok(())
}

fn select_candidate(candidates: &[NamedPhotosPerson]) -> Result<Option<&NamedPhotosPerson>> {
    for (index, candidate) in candidates.iter().enumerate() {
        eprintln!("  {}. {}", index + 1, candidate.name);
    }
    if candidates.len() == 1 {
        return if photos_prompt::confirm(&format!(
            "Link this CRM person to {}? [y/n] ",
            candidates[0].name
        ))? {
            Ok(candidates.first())
        } else {
            Ok(None)
        };
    }
    Ok(photos_prompt::number(
        "Choose the correct Photos person, or 0 to stop: ",
        candidates.len(),
    )?
    .and_then(|index| candidates.get(index)))
}
