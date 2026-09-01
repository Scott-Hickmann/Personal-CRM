use phonenumber::Mode;

pub(crate) fn normalize(value: &str) -> String {
    let value = value.trim();
    if let Some(e164) = valid_e164(value) {
        return e164;
    }
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if value.starts_with("00")
        && let Some(e164) = valid_e164(&format!("+{}", digits.trim_start_matches("00")))
    {
        return e164;
    }
    if value.starts_with('+')
        && let Ok(parsed) = phonenumber::parse(None, value)
    {
        let country_code = parsed.country().code().to_string();
        if let Some(national) = digits.strip_prefix(&country_code)
            && let Some(without_trunk) = national.strip_prefix('0')
            && let Some(e164) = valid_e164(&format!("+{country_code}{without_trunk}"))
        {
            return e164;
        }
    }
    digits
}

fn valid_e164(value: &str) -> Option<String> {
    let number = phonenumber::parse(None, value).ok()?;
    number
        .is_valid()
        .then(|| number.format().mode(Mode::E164).to_string())
        .map(|formatted| formatted.trim_start_matches('+').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_international_and_whatsapp_numbers() {
        assert_eq!(normalize("+1 (415) 328-4536"), "14153284536");
        assert_eq!(normalize("14153284536"), "14153284536");
    }

    #[test]
    fn removes_domestic_trunk_prefix_after_country_code() {
        assert_eq!(normalize("+33 06 51 42 78 44"), "33651427844");
        assert_eq!(normalize("0033 6 51 42 78 44"), "33651427844");
    }

    #[test]
    fn keeps_unparseable_short_codes_as_digits() {
        assert_eq!(normalize("738245"), "738245");
    }
}
