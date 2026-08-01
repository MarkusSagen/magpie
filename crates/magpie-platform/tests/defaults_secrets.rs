//! The built-in secret-ignore regexes and default denylist.

use magpie_platform::{default_app_denylist, default_ignore_regexes};

fn is_ignored(s: &str) -> bool {
    default_ignore_regexes().iter().any(|r| r.is_match(s))
}

#[test]
fn matches_aws_access_key() {
    assert!(is_ignored("AKIAIOSFODNN7EXAMPLE"));
    assert!(is_ignored("key=AKIA1234567890ABCDEF here"));
}

#[test]
fn matches_pem_private_key_header() {
    assert!(is_ignored("-----BEGIN RSA PRIVATE KEY-----"));
    assert!(is_ignored("-----BEGIN OPENSSH PRIVATE KEY-----"));
}

#[test]
fn matches_jwt_shaped_token() {
    let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
    assert!(is_ignored(jwt));
}

#[test]
fn does_not_match_ordinary_text() {
    assert!(!is_ignored("just a normal sentence about AWS and keys"));
    assert!(!is_ignored("BEGIN the meeting at noon"));
    assert!(!is_ignored("https://example.com/path"));
    assert!(!is_ignored("email me@example.io"));
}

#[test]
fn default_denylist_has_known_password_managers() {
    let d = default_app_denylist();
    assert!(d.iter().any(|a| a.contains("1Password")));
    assert!(d.iter().any(|a| a.contains("Bitwarden")));
    assert!(!d.is_empty());
}
