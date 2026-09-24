use regex::Regex;

pub fn default_ignore_regexes() -> Vec<Regex> {
    [
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN [A-Z ]+PRIVATE KEY-----",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        r"^sk-",
        r"^sk_",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("built-in ignore regex must compile"))
    .collect()
}

pub fn default_app_denylist() -> Vec<String> {
    vec![
        "1Password".into(),
        "Bitwarden".into(),
        "KeePassXC".into(),
        "LastPass".into(),
        "Dashlane".into(),
    ]
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

    #[test]
    fn denylist_covers_major_password_managers() {
        let d = default_app_denylist();
        for app in [
            "1Password",
            "Bitwarden",
            "KeePassXC",
            "LastPass",
            "Dashlane",
        ] {
            assert!(d.iter().any(|x| x == app), "missing {app}");
        }
    }

    #[test]
    fn ignore_regexes_match_sk_prefixed_secrets() {
        let res = default_ignore_regexes();
        assert!(res.iter().any(|r| r.is_match("sk-abc123")));
        assert!(res.iter().any(|r| r.is_match("sk_abc123")));
    }

    #[test]
    fn ignore_regexes_match_jwts() {
        let res = default_ignore_regexes();
        // Each dot-separated segment must be at least 10 chars of the
        // base64url alphabet for the JWT regex to match; a short signature
        // (e.g. the 6-char "abc123" one might reach for) does NOT match.
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert!(
            res.iter().any(|r| r.is_match(jwt)),
            "expected a JWT-shaped string to match"
        );
    }

    #[test]
    fn ignore_regexes_match_pem_private_keys() {
        let res = default_ignore_regexes();
        assert!(res
            .iter()
            .any(|r| r.is_match("-----BEGIN RSA PRIVATE KEY-----")));
    }

    #[test]
    fn ignore_regexes_do_not_match_plain_text() {
        let res = default_ignore_regexes();
        let sentence = "The quick brown fox jumps over the lazy dog.";
        assert!(!res.iter().any(|r| r.is_match(sentence)));
    }
}
