use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use uuid::Uuid;

use crate::cli::PhotosReviewArgs;
use crate::commands;
use crate::error::{CrmError, Result};
use crate::face_commands;
use crate::face_matching::QueryFaceprint;
use crate::output::{self, Format};
use crate::photo_links::{self, PhotoReviewPerson};
use crate::photos_import;
use crate::photos_library::{self, PhotosCatalog};
use crate::photos_names;
use crate::photos_prompt;
use crate::repository;

#[derive(Serialize)]
struct ReviewResult {
    people_reviewed: usize,
    photos_people_linked: usize,
    assets_linked: usize,
    deferred: usize,
    not_applicable: usize,
    stopped_early: bool,
}

pub(crate) fn run(format: Format, config_path: PathBuf, mut args: PhotosReviewArgs) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    let person_id = args
        .person
        .as_deref()
        .map(|reference| repository::resolve_person_id(&connection, reference))
        .transpose()?;
    let explicit_library = args.library.is_some();
    let library = photos_library::discover_library(args.library.take())?;
    let catalog = PhotosCatalog::open(&library)?;
    let named_people = catalog.named_people()?;
    let people = photo_links::review_people(&connection, person_id.as_deref())?;
    let targeted = person_id.is_some();
    let mut result = ReviewResult {
        people_reviewed: 0,
        photos_people_linked: 0,
        assets_linked: 0,
        deferred: 0,
        not_applicable: 0,
        stopped_early: false,
    };

    for person in people {
        let state = person
            .link
            .as_ref()
            .map(|link| link.state.as_str())
            .unwrap_or("pending");
        if matches!(state, "asset_linked" | "person_linked") {
            if targeted {
                eprintln!(
                    "{} is already linked; use `crm photos reconcile` if naming is pending.",
                    person.display_name
                );
            }
            continue;
        }
        if !targeted && state == "not_applicable" {
            continue;
        }
        result.people_reviewed += 1;
        eprintln!("\n{} [{}]", person.display_name, state);
        let exact = photos_names::exact_matches(&person.display_name, &named_people);
        if !exact.is_empty() {
            eprintln!(
                "Photos name match{}:",
                if exact.len() == 1 { "" } else { "es" }
            );
            if let Some(candidate) =
                photos_names::choose(&exact, "Link one of these Photos people?", true)?
            {
                photo_links::link_photos_person(
                    &connection,
                    &person.person_id,
                    &candidate.person_uuid,
                    &candidate.name,
                    candidate.key_asset_id.as_deref(),
                )?;
                result.photos_people_linked += 1;
                continue;
            }
        }

        loop {
            match photos_prompt::choice(
                "[l]ink existing Photos person, [p]ick photo, [d]efer, [x]not applicable, [q]uit: ",
                &['l', 'p', 'd', 'x', 'q'],
            )? {
                'l' => {
                    if let Some(candidate) = photos_names::search(&named_people)? {
                        photo_links::link_photos_person(
                            &connection,
                            &person.person_id,
                            &candidate.person_uuid,
                            &candidate.name,
                            candidate.key_asset_id.as_deref(),
                        )?;
                        result.photos_people_linked += 1;
                        break;
                    }
                }
                'p' => {
                    if explicit_library {
                        return Err(CrmError::Photos(
                            "photo import targets the active System Photo Library; omit --library when importing"
                                .into(),
                        ));
                    }
                    let photo = match args.photo.take() {
                        Some(path) => path,
                        None => match photos_import::select_photo(&person.display_name)? {
                            Some(path) => path,
                            None => continue,
                        },
                    };
                    if review_and_import(&connection, &person, &photo)? {
                        result.assets_linked += 1;
                        break;
                    }
                }
                'd' => {
                    photo_links::set_review_state(&connection, &person.person_id, "deferred")?;
                    result.deferred += 1;
                    break;
                }
                'x' => {
                    photo_links::set_review_state(
                        &connection,
                        &person.person_id,
                        "not_applicable",
                    )?;
                    result.not_applicable += 1;
                    break;
                }
                _ => {
                    result.stopped_early = true;
                    break;
                }
            }
        }
        if result.stopped_early {
            break;
        }
    }

    let table = format!(
        "reviewed {}\nPhotos people linked {}\nassets linked {}\ndeferred {}\nnot applicable {}{}",
        result.people_reviewed,
        result.photos_people_linked,
        result.assets_linked,
        result.deferred,
        result.not_applicable,
        if result.stopped_early {
            "\nstopped early"
        } else {
            ""
        },
    );
    output::emit(format, "photos.review", &result, table)
}

fn review_and_import(
    connection: &rusqlite::Connection,
    person: &PhotoReviewPerson,
    photo: &Path,
) -> Result<bool> {
    if !photo.is_file() {
        return Err(CrmError::Photos(format!(
            "photo not found at {}",
            photo.display()
        )));
    }
    let preview = std::env::temp_dir().join(format!("crm-photo-review-{}.png", Uuid::new_v4()));
    let faces = face_commands::detect_faces(photo, &preview)?;
    show_preview(&preview);
    let selected = select_face(&faces)?;
    let _ = fs::remove_file(&preview);
    let Some(selected) = selected else {
        return Ok(false);
    };
    let hash = photos_import::sha256(photo)?;
    let asset_id = if let Some(existing) = photo_links::asset_for_hash(connection, &hash)? {
        if !photos_prompt::confirm("This photo is already imported. Reuse it? [y/n] ")? {
            return Ok(false);
        }
        existing
    } else {
        if !photos_prompt::confirm("Import this photo into the CRM Imports album? [y/n] ")? {
            return Ok(false);
        }
        photos_import::import_photo(photo, &person.person_id)?
    };
    photo_links::link_asset(
        connection,
        &person.person_id,
        &asset_id,
        selected.face_index,
        &selected.bounding_box,
        &hash,
    )
    .map_err(|error| {
        CrmError::Photos(format!(
            "Photos asset {asset_id} is available, but the CRM link could not be saved: {error}"
        ))
    })?;
    eprintln!(
        "Imported and linked. Name {}'s face in Photos, then run `crm photos reconcile`.",
        person.display_name
    );
    Ok(true)
}

fn show_preview(path: &Path) {
    if Command::new("open").arg(path).status().is_err() {
        eprintln!("Preview saved at {}", path.display());
    }
}

fn select_face(faces: &[QueryFaceprint]) -> Result<Option<&QueryFaceprint>> {
    if faces.len() == 1 {
        return if photos_prompt::confirm("Use the detected face labeled 1? [y/n] ")? {
            Ok(faces.first())
        } else {
            Ok(None)
        };
    }
    eprintln!(
        "Detected {} faces; choose the numbered face in the preview.",
        faces.len()
    );
    Ok(
        photos_prompt::number("Face number, or 0 to cancel: ", faces.len())?
            .and_then(|index| faces.get(index)),
    )
}
