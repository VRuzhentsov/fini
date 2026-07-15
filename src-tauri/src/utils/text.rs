pub fn bool_to_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{bool_to_str, parse_bool};

    #[test]
    fn encodes_bools_as_lowercase_text() {
        assert_eq!(bool_to_str(true), "true");
        assert_eq!(bool_to_str(false), "false");
    }

    #[test]
    fn parses_common_boolean_text_values() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));
    }

    #[test]
    fn parses_boolean_text_case_insensitively_and_trims_whitespace() {
        assert_eq!(parse_bool(" TRUE "), Some(true));
        assert_eq!(parse_bool(" Off\n"), Some(false));
    }

    #[test]
    fn rejects_unknown_boolean_text() {
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(parse_bool("enabled"), None);
    }
}
