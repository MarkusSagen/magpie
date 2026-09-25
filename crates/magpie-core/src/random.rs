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

/// A parsed random-generation request recognised from the launcher search box.
/// Parsing is pure and total; the app turns a request into a result by supplying
/// OS entropy (see `magpie-app`).
#[derive(Debug, Clone, PartialEq)]
pub enum RandomRequest {
    Password { len: usize, opts: PasswordOpts },
    Uuid,
    IntRange { lo: i64, hi: i64 },
    FloatRange { lo: f64, hi: f64 },
    Dice { count: u32, sides: i64 },
    Coin,
}

/// The default password length when the user types just `password`.
pub const DEFAULT_PASSWORD_LEN: usize = 20;

/// `NdM` dice spec (e.g. `2d6`, `d20`) → `(count, sides)`. `d20` means one d20.
fn parse_dice_token(tok: &str) -> Option<(u32, i64)> {
    let (a, b) = tok.split_once('d')?;
    let count: u32 = if a.is_empty() { 1 } else { a.parse().ok()? };
    let sides: i64 = b.parse().ok()?;
    if count == 0 || count > 1000 || !(2..=1_000_000).contains(&sides) {
        return None;
    }
    Some((count, sides))
}

/// Recognise a random-generation request in `query`, or `None` if the text is not
/// a generator command. Deliberately strict — every non-numeric token must be a
/// known keyword — so ordinary searches like `password manager` are NOT hijacked.
pub fn parse_random_request(query: &str) -> Option<RandomRequest> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let joined: String = q.split_whitespace().collect();
    let toks: Vec<&str> = q.split_whitespace().collect();

    // Coin flip.
    if matches!(
        joined.as_str(),
        "coin" | "coinflip" | "flipcoin" | "flipacoin" | "headsortails"
    ) {
        return Some(RandomRequest::Coin);
    }
    if toks.first() == Some(&"flip") && (toks.len() == 1 || joined.contains("coin")) {
        return Some(RandomRequest::Coin);
    }

    // UUID / GUID.
    if matches!(
        joined.as_str(),
        "uuid" | "guid" | "uuidv4" | "guidv4" | "uuid4"
    ) {
        return Some(RandomRequest::Uuid);
    }

    // Dice: `2d6`, `d20`, or `roll`/`dice`/`die [NdM]`.
    if let Some((c, s)) = parse_dice_token(&joined) {
        return Some(RandomRequest::Dice { count: c, sides: s });
    }
    if matches!(toks.first(), Some(&"roll") | Some(&"dice") | Some(&"die")) {
        let spec = toks
            .iter()
            .skip(1)
            .find_map(|t| parse_dice_token(t))
            .unwrap_or((1, 6));
        return Some(RandomRequest::Dice {
            count: spec.0,
            sides: spec.1,
        });
    }

    // Password: first token a password keyword; every remaining token must be a
    // length or a recognised option word (else it is an ordinary search).
    let pw_head = matches!(
        toks.first(),
        Some(&"password") | Some(&"passwd") | Some(&"pw")
    ) || (toks.len() >= 2
        && matches!(toks[0], "generate" | "random" | "new")
        && matches!(toks[1], "password" | "passwd" | "pw"));
    if pw_head {
        let start = if matches!(toks[0], "generate" | "random" | "new") {
            2
        } else {
            1
        };
        let mut len = DEFAULT_PASSWORD_LEN;
        // Enabled sets. Once the user names a set explicitly, we switch from the
        // default (all on) to only the named sets; `explicit_sets` marks that flip.
        let mut letters = true;
        let mut digits = true;
        let mut symbols = true;
        let mut explicit_sets = false;
        // Turn on `which` as an explicit set: the first explicit keyword clears the
        // all-on default first (without a dead store on `which` itself).
        let mut enable =
            |which: char, letters: &mut bool, digits: &mut bool, symbols: &mut bool| {
                if !explicit_sets {
                    *letters = false;
                    *digits = false;
                    *symbols = false;
                    explicit_sets = true;
                }
                match which {
                    'l' => *letters = true,
                    'd' => *digits = true,
                    's' => *symbols = true,
                    _ => {}
                }
            };
        let mut i = start;
        while i < toks.len() {
            let t = toks[i];
            if let Ok(n) = t.parse::<usize>() {
                len = n.clamp(4, 256);
            } else if t == "no" && i + 1 < toks.len() {
                match toks[i + 1] {
                    "symbols" | "symbol" | "special" | "punctuation" => symbols = false,
                    "digits" | "numbers" | "numeric" => digits = false,
                    "letters" | "alpha" => letters = false,
                    _ => return None,
                }
                i += 1;
            } else {
                match t {
                    "letters" | "alpha" => enable('l', &mut letters, &mut digits, &mut symbols),
                    "digits" | "numbers" | "numeric" => {
                        enable('d', &mut letters, &mut digits, &mut symbols)
                    }
                    "symbols" | "symbol" | "special" => {
                        enable('s', &mut letters, &mut digits, &mut symbols)
                    }
                    "alnum" | "alphanumeric" => {
                        enable('l', &mut letters, &mut digits, &mut symbols);
                        enable('d', &mut letters, &mut digits, &mut symbols);
                        symbols = false;
                    }
                    "chars" | "char" | "characters" | "long" => {} // filler words
                    _ => return None,
                }
            }
            i += 1;
        }
        return Some(RandomRequest::Password {
            len,
            opts: PasswordOpts {
                letters,
                digits,
                symbols,
            },
        });
    }

    // Numeric range: needs a `random`/`rand`/`rng` keyword. Bare keyword → 1..100.
    let has_rand_kw = toks.iter().any(|t| matches!(*t, "random" | "rand" | "rng"));
    if has_rand_kw {
        let nums: Vec<&str> = toks
            .iter()
            .copied()
            .filter(|t| {
                !matches!(
                    *t,
                    "random" | "rand" | "rng" | "between" | "and" | "to" | "-"
                )
            })
            .collect();
        if nums.is_empty() {
            return Some(RandomRequest::IntRange { lo: 1, hi: 100 });
        }
        // A single `a-b` token, or two separate numeric tokens.
        let pair: Option<(String, String)> = if nums.len() == 1 {
            nums[0]
                .split_once('-')
                .filter(|(a, b)| !a.is_empty() && !b.is_empty())
                .map(|(a, b)| (a.to_string(), b.to_string()))
        } else if nums.len() == 2 {
            Some((nums[0].to_string(), nums[1].to_string()))
        } else {
            None
        };
        if let Some((a, b)) = pair {
            let is_float = a.contains('.') || b.contains('.');
            if is_float {
                if let (Ok(lo), Ok(hi)) = (a.parse::<f64>(), b.parse::<f64>()) {
                    return Some(RandomRequest::FloatRange { lo, hi });
                }
            } else if let (Ok(lo), Ok(hi)) = (a.parse::<i64>(), b.parse::<i64>()) {
                return Some(RandomRequest::IntRange { lo, hi });
            }
            return None;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_password_requests() {
        assert_eq!(
            parse_random_request("password"),
            Some(RandomRequest::Password {
                len: DEFAULT_PASSWORD_LEN,
                opts: PasswordOpts::default()
            })
        );
        assert_eq!(
            parse_random_request("password 32"),
            Some(RandomRequest::Password {
                len: 32,
                opts: PasswordOpts::default()
            })
        );
        // Explicit sets narrow the charset.
        assert_eq!(
            parse_random_request("password 16 letters digits"),
            Some(RandomRequest::Password {
                len: 16,
                opts: PasswordOpts {
                    letters: true,
                    digits: true,
                    symbols: false
                }
            })
        );
        assert_eq!(
            parse_random_request("password no symbols"),
            Some(RandomRequest::Password {
                len: DEFAULT_PASSWORD_LEN,
                opts: PasswordOpts {
                    letters: true,
                    digits: true,
                    symbols: false
                }
            })
        );
        // Ordinary searches are NOT hijacked.
        assert_eq!(parse_random_request("password manager"), None);
        assert_eq!(parse_random_request("my passwords"), None);
    }

    #[test]
    fn parse_uuid_dice_coin() {
        assert_eq!(parse_random_request("uuid"), Some(RandomRequest::Uuid));
        assert_eq!(parse_random_request("GUID"), Some(RandomRequest::Uuid));
        assert_eq!(
            parse_random_request("2d6"),
            Some(RandomRequest::Dice { count: 2, sides: 6 })
        );
        assert_eq!(
            parse_random_request("d20"),
            Some(RandomRequest::Dice {
                count: 1,
                sides: 20
            })
        );
        assert_eq!(
            parse_random_request("roll"),
            Some(RandomRequest::Dice { count: 1, sides: 6 })
        );
        assert_eq!(
            parse_random_request("roll 3d8"),
            Some(RandomRequest::Dice { count: 3, sides: 8 })
        );
        assert_eq!(parse_random_request("coin flip"), Some(RandomRequest::Coin));
        assert_eq!(parse_random_request("flip"), Some(RandomRequest::Coin));
    }

    #[test]
    fn parse_ranges() {
        assert_eq!(
            parse_random_request("random 1-100"),
            Some(RandomRequest::IntRange { lo: 1, hi: 100 })
        );
        assert_eq!(
            parse_random_request("rand 5 10"),
            Some(RandomRequest::IntRange { lo: 5, hi: 10 })
        );
        assert_eq!(
            parse_random_request("random between 1 and 6"),
            Some(RandomRequest::IntRange { lo: 1, hi: 6 })
        );
        assert_eq!(
            parse_random_request("random"),
            Some(RandomRequest::IntRange { lo: 1, hi: 100 })
        );
        match parse_random_request("random 0.0-1.0") {
            Some(RandomRequest::FloatRange { lo, hi }) => {
                assert_eq!(lo, 0.0);
                assert_eq!(hi, 1.0);
            }
            other => panic!("expected float range, got {other:?}"),
        }
        // Not a generator command.
        assert_eq!(parse_random_request("random thoughts"), None);
    }

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
