//! Random generators (Phase 13). The core is **pure**: every function takes the
//! entropy it needs as an argument (a byte slice or a `u64`), so it is fully
//! deterministic and table-driven testable. The app supplies real CSPRNG bytes
//! (OS `getrandom`) at call time. Generated passwords are never written to
//! history unless the user pastes them (enforced at the call site).

/// Character sets a password can draw from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordOpts {
    pub letters: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for PasswordOpts {
    fn default() -> Self {
        Self {
            letters: true,
            digits: true,
            symbols: true,
        }
    }
}

fn charset(opts: &PasswordOpts) -> Vec<u8> {
    let mut cs = Vec::new();
    if opts.letters {
        cs.extend(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }
    if opts.digits {
        cs.extend(b"0123456789");
    }
    if opts.symbols {
        cs.extend(b"!@#$%^&*()-_=+[]{};:,.?/");
    }
    if cs.is_empty() {
        cs.extend(b"abcdefghijklmnopqrstuvwxyz");
    }
    cs
}

/// A password of `len` chars, drawing each char from the enabled sets by indexing
/// the charset with successive bytes of `entropy` (wrapping if short — callers
/// should pass ≥ `len` bytes for full strength).
pub fn password(entropy: &[u8], len: usize, opts: &PasswordOpts) -> String {
    if len == 0 {
        return String::new();
    }
    let cs = charset(opts);
    (0..len)
        .map(|i| {
            let b = if entropy.is_empty() {
                (i as u8).wrapping_mul(31)
            } else {
                entropy[i % entropy.len()]
            };
            cs[b as usize % cs.len()] as char
        })
        .collect()
}

/// A v4 UUID from 16 random bytes (version + variant nibbles forced per RFC 4122).
pub fn uuid_v4(bytes: [u8; 16]) -> String {
    let mut b = bytes;
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// A uniform integer in `[lo, hi]` (inclusive) from a random `u64`. `lo`/`hi` are
/// ordered for you; an empty range returns `lo`.
pub fn int_in_range(rand: u64, lo: i64, hi: i64) -> i64 {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let span = (hi - lo) as u128 + 1;
    lo + (rand as u128 % span) as i64
}

/// A float in `[lo, hi)` from a random `u64` (53-bit mantissa precision).
pub fn float_in_range(rand: u64, lo: f64, hi: f64) -> f64 {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let unit = (rand >> 11) as f64 / (1u64 << 53) as f64; // [0,1)
    lo + unit * (hi - lo)
}

/// A word passphrase (`correct horse battery staple`) selected from `wordlist`
/// using `indices` (one per word, taken modulo the list length), joined by `sep`.
/// Pure — the app supplies the random indices and the bundled EFF wordlist.
pub fn passphrase(indices: &[usize], wordlist: &[&str], sep: &str) -> String {
    if wordlist.is_empty() {
        return String::new();
    }
    indices
        .iter()
        .map(|&i| wordlist[i % wordlist.len()])
        .collect::<Vec<_>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_length_and_charset() {
        let e: Vec<u8> = (0..64).collect();
        let p = password(&e, 20, &PasswordOpts::default());
        assert_eq!(p.chars().count(), 20);
        // Deterministic for the same entropy.
        assert_eq!(p, password(&e, 20, &PasswordOpts::default()));
        // Letters-only stays alphabetic.
        let alpha = password(
            &e,
            30,
            &PasswordOpts {
                letters: true,
                digits: false,
                symbols: false,
            },
        );
        assert!(alpha.chars().all(|c| c.is_ascii_alphabetic()));
    }

    #[test]
    fn uuid_v4_format_and_version() {
        let u = uuid_v4([0xff; 16]);
        assert_eq!(u.len(), 36);
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert_eq!(&u[14..15], "4"); // version nibble
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b")); // variant
    }

    #[test]
    fn int_range_is_inclusive_and_bounded() {
        for r in 0u64..1000 {
            let n = int_in_range(r, 1, 6);
            assert!((1..=6).contains(&n));
        }
        assert_eq!(int_in_range(0, 5, 5), 5); // empty range
        assert_eq!(int_in_range(0, 10, 1), 1); // lo/hi reordered → min
    }

    #[test]
    fn float_range_is_bounded() {
        for r in [0u64, 1, u64::MAX / 2, u64::MAX] {
            let f = float_in_range(r, 0.5, 2.0);
            assert!((0.5..2.0).contains(&f) || f == 0.5);
        }
    }

    #[test]
    fn passphrase_joins_words() {
        let list = ["correct", "horse", "battery", "staple"];
        assert_eq!(
            passphrase(&[0, 1, 2, 3], &list, "-"),
            "correct-horse-battery-staple"
        );
        // Indices wrap modulo the list length.
        assert_eq!(passphrase(&[4, 5], &list, " "), "correct horse");
    }
}
