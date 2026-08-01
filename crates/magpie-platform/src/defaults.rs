use regex::Regex;

pub fn default_ignore_regexes() -> Vec<Regex> {
    [
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN [A-Z ]+PRIVATE KEY-----",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("built-in ignore regex must compile"))
    .collect()
}

pub fn default_app_denylist() -> Vec<String> {
    vec!["1Password".into(), "Bitwarden".into(), "KeePassXC".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_regexes_compile_and_match_known_secrets() {
        let res = default_ignore_regexes();
        assert!(!res.is_empty());
        assert!(res.iter().any(|r| r.is_match("AKIAABCDEFGHIJKLMNOP")));
    }

    #[test]
    fn denylist_nonempty() {
        assert!(!default_app_denylist().is_empty());
    }
}
