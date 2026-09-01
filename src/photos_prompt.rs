use std::io::{self, Write};

use crate::error::{CrmError, Result};

pub(crate) fn line(message: &str) -> Result<Option<String>> {
    eprint!("{message}");
    io::stderr()
        .flush()
        .map_err(|error| CrmError::Photos(format!("could not display prompt: {error}")))?;
    let mut input = String::new();
    let read = io::stdin()
        .read_line(&mut input)
        .map_err(|error| CrmError::Photos(format!("could not read response: {error}")))?;
    if read == 0 {
        Ok(None)
    } else {
        Ok(Some(input.trim().to_owned()))
    }
}

pub(crate) fn choice(message: &str, allowed: &[char]) -> Result<char> {
    loop {
        let Some(input) = line(message)? else {
            return Ok('q');
        };
        if let Some(choice) = input
            .chars()
            .next()
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| allowed.contains(value))
        {
            return Ok(choice);
        }
        eprintln!("Enter one of: {}", allowed.iter().collect::<String>());
    }
}

pub(crate) fn confirm(message: &str) -> Result<bool> {
    Ok(choice(message, &['y', 'n'])? == 'y')
}

pub(crate) fn number(message: &str, maximum: usize) -> Result<Option<usize>> {
    loop {
        let Some(input) = line(message)? else {
            return Ok(None);
        };
        match input.parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(value) if value <= maximum => return Ok(Some(value - 1)),
            _ => eprintln!("Enter 1-{maximum}, or 0 to cancel."),
        }
    }
}
