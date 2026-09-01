use crate::error::Result;
use crate::photos_library::NamedPhotosPerson;
use crate::photos_prompt;

pub(crate) fn exact_matches<'a>(
    name: &str,
    people: &'a [NamedPhotosPerson],
) -> Vec<&'a NamedPhotosPerson> {
    let normalized = normalize(name);
    people
        .iter()
        .filter(|person| normalize(&person.name) == normalized)
        .collect()
}

pub(crate) fn search(people: &[NamedPhotosPerson]) -> Result<Option<&NamedPhotosPerson>> {
    let Some(search) = photos_prompt::line("Photos person name: ")? else {
        return Ok(None);
    };
    let search = normalize(&search);
    let matches = people
        .iter()
        .filter(|person| normalize(&person.name).contains(&search))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        eprintln!("No named Photos people matched.");
        return Ok(None);
    }
    if matches.len() > 20 {
        eprintln!("{} matches; enter a more specific name.", matches.len());
        return Ok(None);
    }
    choose(&matches, "Choose a Photos person", false)
}

pub(crate) fn choose<'a>(
    people: &[&'a NamedPhotosPerson],
    message: &str,
    confirm_single: bool,
) -> Result<Option<&'a NamedPhotosPerson>> {
    for (index, person) in people.iter().enumerate() {
        eprintln!("  {}. {}", index + 1, person.name);
    }
    if people.len() == 1 && confirm_single {
        return if photos_prompt::confirm(&format!("{message} [y/n] "))? {
            Ok(people.first().copied())
        } else {
            Ok(None)
        };
    }
    Ok(photos_prompt::number(
        &format!("{message}; enter 1-{}, or 0 to cancel: ", people.len()),
        people.len(),
    )?
    .and_then(|index| people.get(index).copied()))
}

fn normalize(name: &str) -> String {
    name.split_whitespace()
        .flat_map(str::chars)
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_ignores_case_and_whitespace() {
        let people = vec![NamedPhotosPerson {
            person_uuid: "1".into(),
            name: "Ada  Lovelace".into(),
            key_asset_id: None,
        }];
        assert_eq!(exact_matches(" ada lovelace ", &people).len(), 1);
    }
}
