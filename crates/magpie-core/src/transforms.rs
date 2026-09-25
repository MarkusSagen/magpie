//! Transforms (⌘T) — pure, dependency-free text transforms for the clipboard palette.
//!
//! No I/O, no `Store` dependency, no new crates: base64, percent-encoding, HTML
//! entities, SHA-256 and MD5 are all hand-rolled here per `CLAUDE.md`'s "no new
//! crates without asking" rule. QR code generation is the one exception the spec
//! calls out as needing a gated crate — `qr()` below is a stub that always errors.
//!
//! See `docs/superpowers/specs/2026-09-24-magpie-transforms-design.md` for the
//! full design (option tables, edge cases, UI wiring — none of which lives here).

use std::fmt;

/// Maximum input size accepted by any transform (2 MiB) — keeps the palette fast
/// on the UI thread. See the spec's "Very large input" edge case.
pub const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// QR's own, much smaller hard cap: binary payload capacity at max QR version /
/// lowest error-correction level.
pub const MAX_QR_PAYLOAD_BYTES: usize = 2953;

// ---------------------------------------------------------------------------
// Enums / option types (shape mirrors the spec's sketch)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    SqlInClause,
    WrapJoin,
    SortUniqueTrim,
    Base64,
    Url,
    HtmlEntities,
    Hash,
    Qr,
    Count,
    // Case conversions (Phase 13) — index into `case::all_cases`.
    CaseCamel,
    CasePascal,
    CaseSnake,
    CaseKebab,
    CaseScreaming,
    CaseTrain,
    CaseTitle,
    CaseUpper,
    CaseLower,
}

/// Apply case style `n` (0..9, order of `case::all_cases`) to `input`.
fn case_n(input: &str, n: usize) -> Result<String, TransformError> {
    Ok(crate::case::all_cases(input)
        .get(n)
        .map(|(_, v)| v.clone())
        .unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    Single,
    Double,
    None,
}

impl Quote {
    fn wrap(self, s: &str) -> String {
        match self {
            Quote::Single => format!("'{s}'"),
            Quote::Double => format!("\"{s}\""),
            Quote::None => s.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Parens,
    Brackets,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Encode,
    Decode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base64Alphabet {
    Standard,
    UrlSafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlScope {
    Component,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Md5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexCase {
    Lower,
    Upper,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlInClauseOptions {
    pub quote: Quote,
    pub separator: String,
    pub wrap: Wrap,
}

impl Default for SqlInClauseOptions {
    fn default() -> Self {
        Self {
            quote: Quote::Single,
            separator: ", ".to_string(),
            wrap: Wrap::Parens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapJoinOptions {
    pub quote: Quote,
    pub separator: String,
    pub prefix: String,
    pub suffix: String,
}

impl Default for WrapJoinOptions {
    fn default() -> Self {
        Self {
            quote: Quote::None,
            separator: ", ".to_string(),
            prefix: String::new(),
            suffix: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortUniqueTrimOptions {
    pub sort: Option<SortOrder>,
    pub unique: bool,
    pub trim: bool,
    pub case_sensitive: bool,
}

impl Default for SortUniqueTrimOptions {
    fn default() -> Self {
        Self {
            sort: None,
            unique: false,
            trim: true,
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base64Options {
    pub mode: Mode,
    pub alphabet: Base64Alphabet,
    pub padding: bool,
}

impl Default for Base64Options {
    fn default() -> Self {
        Self {
            mode: Mode::Encode,
            alphabet: Base64Alphabet::Standard,
            padding: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlOptions {
    pub mode: Mode,
    pub scope: UrlScope,
}

impl Default for UrlOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Encode,
            scope: UrlScope::Component,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HtmlEntityOptions {
    pub mode: Mode,
}

impl Default for HtmlEntityOptions {
    fn default() -> Self {
        Self { mode: Mode::Encode }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashOptions {
    pub algorithm: HashAlgorithm,
    pub case: HexCase,
}

impl Default for HashOptions {
    fn default() -> Self {
        Self {
            algorithm: HashAlgorithm::Sha256,
            case: HexCase::Lower,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformError {
    EmptyInput,
    InvalidBase64(String),
    InvalidPercentEncoding(String),
    DecodedBytesNotUtf8,
    InputTooLarge { len: usize, max: usize },
    QrPayloadTooLarge { len: usize, max: usize },
    NotImplemented(&'static str),
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransformError::EmptyInput => write!(f, "input is empty"),
            TransformError::InvalidBase64(msg) => write!(f, "invalid base64: {msg}"),
            TransformError::InvalidPercentEncoding(msg) => {
                write!(f, "invalid percent-encoding: {msg}")
            }
            TransformError::DecodedBytesNotUtf8 => {
                write!(f, "decoded bytes are not valid UTF-8")
            }
            TransformError::InputTooLarge { len, max } => {
                write!(f, "input too large: {len} bytes (max {max} bytes)")
            }
            TransformError::QrPayloadTooLarge { len, max } => {
                write!(f, "QR payload too large: {len} bytes (max {max} bytes)")
            }
            TransformError::NotImplemented(what) => write!(f, "{what} is not implemented yet"),
        }
    }
}

impl std::error::Error for TransformError {}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn check_size(input: &str) -> Result<(), TransformError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(TransformError::InputTooLarge {
            len: input.len(),
            max: MAX_INPUT_BYTES,
        });
    }
    Ok(())
}

/// Normalize `\r\n` to `\n` before line-splitting, per the spec's "mixed line
/// endings" edge case. Only used by line-based transforms; never applied to
/// whole-blob transforms (Base64/Hash/etc.) or to Count's byte/char totals.
fn normalize_line_endings(input: &str) -> String {
    input.replace("\r\n", "\n")
}

// ---------------------------------------------------------------------------
// 1. SQL IN clause
// ---------------------------------------------------------------------------

pub fn sql_in_clause(input: &str, opts: &SqlInClauseOptions) -> Result<String, TransformError> {
    check_size(input)?;
    let normalized = normalize_line_endings(input);
    let items: Vec<String> = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| opts.quote.wrap(line))
        .collect();
    let joined = items.join(&opts.separator);
    Ok(match opts.wrap {
        Wrap::Parens => format!("({joined})"),
        Wrap::Brackets => format!("[{joined}]"),
        Wrap::None => joined,
    })
}

// ---------------------------------------------------------------------------
// 2. Wrap and join lines
// ---------------------------------------------------------------------------

pub fn wrap_join(input: &str, opts: &WrapJoinOptions) -> Result<String, TransformError> {
    check_size(input)?;
    let normalized = normalize_line_endings(input);
    let items: Vec<String> = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| opts.quote.wrap(line))
        .collect();
    let joined = items.join(&opts.separator);
    Ok(format!("{}{joined}{}", opts.prefix, opts.suffix))
}

// ---------------------------------------------------------------------------
// 3. Sort · unique · trim
// ---------------------------------------------------------------------------

pub fn sort_unique_trim(
    input: &str,
    opts: &SortUniqueTrimOptions,
) -> Result<String, TransformError> {
    check_size(input)?;
    let normalized = normalize_line_endings(input);
    let mut lines: Vec<String> = normalized
        .lines()
        .map(|line| {
            if opts.trim {
                line.trim().to_string()
            } else {
                line.to_string()
            }
        })
        .collect();

    if opts.unique {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut deduped = Vec::with_capacity(lines.len());
        for line in lines {
            let key = if opts.case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };
            if seen.insert(key) {
                deduped.push(line);
            }
        }
        lines = deduped;
    }

    if let Some(order) = opts.sort {
        lines.sort_by(|a, b| {
            let cmp = if opts.case_sensitive {
                a.cmp(b)
            } else {
                a.to_lowercase().cmp(&b.to_lowercase())
            };
            match order {
                SortOrder::Ascending => cmp,
                SortOrder::Descending => cmp.reverse(),
            }
        });
    }

    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// 4. Base64
// ---------------------------------------------------------------------------

const BASE64_STANDARD: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_URL_SAFE: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64_alphabet_table(alphabet: Base64Alphabet) -> &'static [u8; 64] {
    match alphabet {
        Base64Alphabet::Standard => BASE64_STANDARD,
        Base64Alphabet::UrlSafe => BASE64_URL_SAFE,
    }
}

fn base64_encode_bytes(data: &[u8], alphabet: Base64Alphabet, padding: bool) -> String {
    let table = base64_alphabet_table(alphabet);
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut chunks = data.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(table[((n >> 18) & 0x3F) as usize] as char);
        out.push(table[((n >> 12) & 0x3F) as usize] as char);
        out.push(table[((n >> 6) & 0x3F) as usize] as char);
        out.push(table[(n & 0x3F) as usize] as char);
    }
    match chunks.remainder() {
        [] => {}
        [b0] => {
            let n = (*b0 as u32) << 16;
            out.push(table[((n >> 18) & 0x3F) as usize] as char);
            out.push(table[((n >> 12) & 0x3F) as usize] as char);
            if padding {
                out.push_str("==");
            }
        }
        [b0, b1] => {
            let n = ((*b0 as u32) << 16) | ((*b1 as u32) << 8);
            out.push(table[((n >> 18) & 0x3F) as usize] as char);
            out.push(table[((n >> 12) & 0x3F) as usize] as char);
            out.push(table[((n >> 6) & 0x3F) as usize] as char);
            if padding {
                out.push('=');
            }
        }
        _ => unreachable!("chunks_exact(3) remainder is always 0, 1 or 2 bytes"),
    }
    out
}

fn base64_decode_bytes(input: &str, alphabet: Base64Alphabet) -> Result<Vec<u8>, TransformError> {
    let table = base64_alphabet_table(alphabet);
    let mut reverse = [0xFFu8; 256];
    for (value, &byte) in table.iter().enumerate() {
        reverse[byte as usize] = value as u8;
    }

    let bytes = input.as_bytes();
    let mut data_len = bytes.len();
    while data_len > 0 && bytes[data_len - 1] == b'=' {
        data_len -= 1;
    }
    let data = &bytes[..data_len];

    if data.len() % 4 == 1 {
        return Err(TransformError::InvalidBase64(
            "input length is not a multiple of 4 (and no valid unpadded length)".to_string(),
        ));
    }

    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for (i, &b) in data.iter().enumerate() {
        let v = reverse[b as usize];
        if v == 0xFF {
            return Err(TransformError::InvalidBase64(format!(
                "invalid character '{}' at byte {i}",
                b as char
            )));
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

pub fn base64_transform(input: &str, opts: &Base64Options) -> Result<String, TransformError> {
    check_size(input)?;
    if input.is_empty() {
        return Ok(String::new());
    }
    match opts.mode {
        Mode::Encode => Ok(base64_encode_bytes(
            input.as_bytes(),
            opts.alphabet,
            opts.padding,
        )),
        Mode::Decode => {
            let bytes = base64_decode_bytes(input, opts.alphabet)?;
            String::from_utf8(bytes).map_err(|_| TransformError::DecodedBytesNotUtf8)
        }
    }
}

// ---------------------------------------------------------------------------
// 5. URL encode / decode
// ---------------------------------------------------------------------------

const PERCENT_HEX: &[u8; 16] = b"0123456789ABCDEF";

fn is_component_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
        )
}

fn is_full_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b';' | b','
                | b'/'
                | b'?'
                | b':'
                | b'@'
                | b'&'
                | b'='
                | b'+'
                | b'$'
                | b'-'
                | b'_'
                | b'.'
                | b'!'
                | b'~'
                | b'*'
                | b'\''
                | b'('
                | b')'
                | b'#'
                | b'['
                | b']'
        )
}

fn percent_encode(input: &str, is_safe: fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if is_safe(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(PERCENT_HEX[(b >> 4) as usize] as char);
            out.push(PERCENT_HEX[(b & 0x0F) as usize] as char);
        }
    }
    out
}

fn hex_digit_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decode `%XX` sequences; `+` becomes a literal space (decode-only
/// convenience per the spec — encode never turns a space into `+`).
fn percent_decode(input: &str) -> Result<Vec<u8>, TransformError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 1 >= bytes.len() || i + 2 >= bytes.len() {
                    return Err(TransformError::InvalidPercentEncoding(format!(
                        "truncated percent-sequence at byte {i}"
                    )));
                }
                let (hi, lo) = (hex_digit_value(bytes[i + 1]), hex_digit_value(bytes[i + 2]));
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    }
                    _ => {
                        return Err(TransformError::InvalidPercentEncoding(format!(
                            "non-hex digits after '%' at byte {i}"
                        )));
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Ok(out)
}

pub fn url_transform(input: &str, opts: &UrlOptions) -> Result<String, TransformError> {
    check_size(input)?;
    match opts.mode {
        Mode::Encode => {
            let is_safe: fn(u8) -> bool = match opts.scope {
                UrlScope::Component => is_component_safe,
                UrlScope::Full => is_full_safe,
            };
            Ok(percent_encode(input, is_safe))
        }
        Mode::Decode => {
            if input.is_empty() {
                return Ok(String::new());
            }
            let bytes = percent_decode(input)?;
            String::from_utf8(bytes).map_err(|_| TransformError::DecodedBytesNotUtf8)
        }
    }
}

// ---------------------------------------------------------------------------
// 6. HTML entities
// ---------------------------------------------------------------------------

const HTML_NAMED_ENTITIES: [(&str, char); 4] = [
    ("&amp;", '&'),
    ("&lt;", '<'),
    ("&gt;", '>'),
    ("&quot;", '"'),
];

fn html_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Matches one entity at the start of `s` (which must start with `&`). Returns
/// the decoded char and the byte length of the matched entity, or `None` if
/// `s` doesn't start with a recognized named or numeric entity (in which case
/// the caller leaves the `&` as literal text — see the spec's decode rules).
fn match_html_entity(s: &str) -> Option<(char, usize)> {
    debug_assert!(s.starts_with('&'));

    for (name, ch) in HTML_NAMED_ENTITIES {
        if s.starts_with(name) {
            return Some((ch, name.len()));
        }
    }

    let after_amp = &s[1..];
    let after_hash = after_amp.strip_prefix('#')?;
    let (is_hex, digits_part) = match after_hash
        .strip_prefix('x')
        .or_else(|| after_hash.strip_prefix('X'))
    {
        Some(rest) => (true, rest),
        None => (false, after_hash),
    };
    let digit_count = digits_part
        .bytes()
        .take_while(|&b| {
            if is_hex {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        })
        .count();
    if digit_count == 0 || digits_part.as_bytes().get(digit_count) != Some(&b';') {
        return None;
    }
    let digits = &digits_part[..digit_count];
    let code = if is_hex {
        u32::from_str_radix(digits, 16).ok()?
    } else {
        digits.parse().ok()?
    };
    let ch = char::from_u32(code)?;
    let prefix_len = 1 + 1 + usize::from(is_hex); // '&' + '#' + optional 'x'
    Some((ch, prefix_len + digit_count + 1)) // + 1 for the trailing ';'
}

fn html_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input.as_bytes()[i] == b'&' {
            if let Some((ch, len)) = match_html_entity(&input[i..]) {
                out.push(ch);
                i += len;
                continue;
            }
        }
        let c = input[i..]
            .chars()
            .next()
            .expect("i < input.len() so a char remains");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

pub fn html_entities(input: &str, opts: &HtmlEntityOptions) -> Result<String, TransformError> {
    check_size(input)?;
    match opts.mode {
        Mode::Encode => Ok(html_encode(input)),
        Mode::Decode => Ok(html_decode(input)),
    }
}

// ---------------------------------------------------------------------------
// 7. Hash (SHA-256, MD5) — hand-rolled
// ---------------------------------------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word_bytes) in chunk.chunks_exact(4).take(16).enumerate() {
            w[i] = u32::from_be_bytes(word_bytes.try_into().expect("chunks_exact(4)"));
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let (mut a0, mut b0, mut c0, mut d0): (u32, u32, u32, u32) =
        (0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476);

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word_bytes) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes(word_bytes.try_into().expect("chunks_exact(4)"));
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | ((!b) & d), i)
            } else if i < 32 {
                ((d & b) | ((!d) & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | (!d)), (7 * i) % 16)
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

fn to_hex(bytes: &[u8], case: HexCase) -> String {
    let table: &[u8; 16] = match case {
        HexCase::Lower => b"0123456789abcdef",
        HexCase::Upper => b"0123456789ABCDEF",
    };
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(table[(b >> 4) as usize] as char);
        s.push(table[(b & 0x0F) as usize] as char);
    }
    s
}

pub fn hash(input: &str, opts: &HashOptions) -> Result<String, TransformError> {
    if input.is_empty() {
        return Err(TransformError::EmptyInput);
    }
    check_size(input)?;
    let digest: Vec<u8> = match opts.algorithm {
        HashAlgorithm::Sha256 => sha256(input.as_bytes()).to_vec(),
        HashAlgorithm::Md5 => md5(input.as_bytes()).to_vec(),
    };
    Ok(to_hex(&digest, opts.case))
}

// ---------------------------------------------------------------------------
// 8. QR code — gated. No QR crate has been added; this always errors.
// ---------------------------------------------------------------------------

/// Stub: QR generation needs Reed–Solomon error correction and the full
/// version/mask-selection table, which isn't reasonable to hand-roll. Per the
/// spec's QR code gate, this ships without QR until a crate (`qrcode` or
/// `fast_qr`) is explicitly approved. Always returns an error; never panics.
pub fn qr(input: &str) -> Result<String, TransformError> {
    if input.is_empty() {
        return Err(TransformError::EmptyInput);
    }
    if input.len() > MAX_QR_PAYLOAD_BYTES {
        return Err(TransformError::QrPayloadTooLarge {
            len: input.len(),
            max: MAX_QR_PAYLOAD_BYTES,
        });
    }
    Err(TransformError::NotImplemented(
        "QR code generation (needs a gated crate — see the transforms design spec)",
    ))
}

// ---------------------------------------------------------------------------
// 9. Count
// ---------------------------------------------------------------------------

pub fn count(input: &str) -> Result<String, TransformError> {
    check_size(input)?;
    let chars = input.chars().count();
    let bytes = input.len();
    let words = input.split_whitespace().count();
    let lines = input.lines().count();
    Ok(format!(
        "Characters: {chars} ({bytes} bytes)\nWords: {words}\nLines: {lines}\nBytes: {bytes}"
    ))
}

/// The transform catalogue in UI order (matches the ⌘T palette / spec grouping).
pub const KINDS: [TransformKind; 18] = [
    TransformKind::SqlInClause,
    TransformKind::WrapJoin,
    TransformKind::SortUniqueTrim,
    TransformKind::Base64,
    TransformKind::Url,
    TransformKind::HtmlEntities,
    TransformKind::Hash,
    TransformKind::Count,
    TransformKind::Qr,
    TransformKind::CaseCamel,
    TransformKind::CasePascal,
    TransformKind::CaseSnake,
    TransformKind::CaseKebab,
    TransformKind::CaseScreaming,
    TransformKind::CaseTrain,
    TransformKind::CaseTitle,
    TransformKind::CaseUpper,
    TransformKind::CaseLower,
];

/// Apply a transform with its default options (UI convenience dispatcher).
pub fn run(kind: TransformKind, input: &str) -> Result<String, TransformError> {
    match kind {
        TransformKind::SqlInClause => sql_in_clause(input, &SqlInClauseOptions::default()),
        TransformKind::WrapJoin => wrap_join(input, &WrapJoinOptions::default()),
        TransformKind::SortUniqueTrim => sort_unique_trim(input, &SortUniqueTrimOptions::default()),
        TransformKind::Base64 => base64_transform(input, &Base64Options::default()),
        TransformKind::Url => url_transform(input, &UrlOptions::default()),
        TransformKind::HtmlEntities => html_entities(input, &HtmlEntityOptions::default()),
        TransformKind::Hash => hash(input, &HashOptions::default()),
        TransformKind::Qr => qr(input),
        TransformKind::Count => count(input),
        TransformKind::CaseCamel => case_n(input, 0),
        TransformKind::CasePascal => case_n(input, 1),
        TransformKind::CaseSnake => case_n(input, 2),
        TransformKind::CaseKebab => case_n(input, 3),
        TransformKind::CaseScreaming => case_n(input, 4),
        TransformKind::CaseTrain => case_n(input, 5),
        TransformKind::CaseTitle => case_n(input, 6),
        TransformKind::CaseUpper => case_n(input, 7),
        TransformKind::CaseLower => case_n(input, 8),
    }
}

/// Result string for the palette row at `i` (error rendered as text, never panics).
pub fn run_index_display(i: usize, input: &str) -> String {
    match KINDS.get(i) {
        Some(k) => match run(*k, input) {
            Ok(s) => s,
            Err(e) => format!("⚠ {e}"),
        },
        None => String::new(),
    }
}

/// A single-line preview of the palette input for the header chip: the text
/// flattened to one line (newlines → spaces), truncated, with a `· N lines`
/// suffix when the input spans multiple lines. E.g. `banana apple apple … · 4 lines`.
pub fn input_summary(input: &str) -> String {
    if input.trim().is_empty() {
        return "clipboard entry".to_string();
    }
    let lines = input.lines().filter(|l| !l.trim().is_empty()).count();
    let flat = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = flat.chars().take(40).collect();
    if flat.chars().count() > 40 {
        preview.push('…');
    }
    if lines > 1 {
        format!("{preview}  ·  {lines} lines")
    } else {
        preview
    }
}

/// A one-line stats summary of applying transform `i` to `input`, for the palette
/// footer: e.g. `4 lines in · 1 line out · 32 chars`. Counts non-empty logical
/// lines on the input side and characters on the output side.
pub fn run_index_stats(i: usize, input: &str) -> String {
    let lines_in = input.lines().filter(|l| !l.trim().is_empty()).count();
    let out = run_index_display(i, input);
    let lines_out = out.lines().count().max(if out.is_empty() { 0 } else { 1 });
    let chars_out = out.chars().count();
    let li = if lines_in == 1 { "line" } else { "lines" };
    let lo = if lines_out == 1 { "line" } else { "lines" };
    let ch = if chars_out == 1 { "char" } else { "chars" };
    format!("{lines_in} {li} in · {lines_out} {lo} out · {chars_out} {ch}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // -- 1. SQL IN clause ---------------------------------------------------

    #[test]
    fn sql_in_clause_golden_cases() {
        let cases: &[(&str, SqlInClauseOptions, &str)] = &[
            (
                "1042\n1043\n1057\n1101",
                SqlInClauseOptions::default(),
                "('1042', '1043', '1057', '1101')",
            ),
            ("", SqlInClauseOptions::default(), "()"),
            ("only", SqlInClauseOptions::default(), "('only')"),
            ("a\n\nb\n\n", SqlInClauseOptions::default(), "('a', 'b')"),
            (" a \n b ", SqlInClauseOptions::default(), "('a', 'b')"),
            ("a\r\nb\r\n", SqlInClauseOptions::default(), "('a', 'b')"),
            (
                "café\nnaïve",
                SqlInClauseOptions::default(),
                "('café', 'naïve')",
            ),
            (
                "a\nb",
                SqlInClauseOptions {
                    quote: Quote::Double,
                    separator: "|".to_string(),
                    wrap: Wrap::Brackets,
                },
                "[\"a\"|\"b\"]",
            ),
            (
                "a\nb",
                SqlInClauseOptions {
                    quote: Quote::None,
                    separator: ",".to_string(),
                    wrap: Wrap::None,
                },
                "a,b",
            ),
        ];
        for (input, opts, expected) in cases {
            assert_eq!(
                sql_in_clause(input, opts).unwrap(),
                *expected,
                "input={input:?}"
            );
        }
    }

    #[test]
    fn sql_in_clause_rejects_oversized_input() {
        let huge = "a".repeat(MAX_INPUT_BYTES + 1);
        let err = sql_in_clause(&huge, &SqlInClauseOptions::default()).unwrap_err();
        assert!(matches!(err, TransformError::InputTooLarge { .. }));
    }

    // -- 2. Wrap and join lines ----------------------------------------------

    #[test]
    fn wrap_join_golden_cases() {
        let cases: &[(&str, WrapJoinOptions, &str)] = &[
            ("a\nb", WrapJoinOptions::default(), "a, b"),
            ("", WrapJoinOptions::default(), ""),
            (
                "a\nb\nc",
                WrapJoinOptions {
                    quote: Quote::Single,
                    separator: " AND ".to_string(),
                    prefix: "WHERE x = ".to_string(),
                    suffix: ";".to_string(),
                },
                "WHERE x = 'a' AND 'b' AND 'c';",
            ),
            ("a\n\nb\n", WrapJoinOptions::default(), "a, b"),
            ("a\r\nb\r\n", WrapJoinOptions::default(), "a, b"),
        ];
        for (input, opts, expected) in cases {
            assert_eq!(
                wrap_join(input, opts).unwrap(),
                *expected,
                "input={input:?}"
            );
        }
    }

    // -- 3. Sort · unique · trim ---------------------------------------------

    #[test]
    fn sort_unique_trim_golden_cases() {
        assert_eq!(
            sort_unique_trim("", &SortUniqueTrimOptions::default()).unwrap(),
            ""
        );

        // Defaults: trim only, nothing else.
        assert_eq!(
            sort_unique_trim(
                "banana\napple\n Banana \napple",
                &SortUniqueTrimOptions::default()
            )
            .unwrap(),
            "banana\napple\nBanana\napple"
        );

        // Sort ascending + unique + trim, case-insensitive.
        assert_eq!(
            sort_unique_trim(
                "banana\napple\n Banana \napple",
                &SortUniqueTrimOptions {
                    sort: Some(SortOrder::Ascending),
                    unique: true,
                    trim: true,
                    case_sensitive: false,
                }
            )
            .unwrap(),
            "apple\nbanana"
        );

        // Unique, case-sensitive: "Apple" and "apple" are distinct.
        assert_eq!(
            sort_unique_trim(
                "Apple\napple\nApple",
                &SortUniqueTrimOptions {
                    sort: None,
                    unique: true,
                    trim: true,
                    case_sensitive: true,
                }
            )
            .unwrap(),
            "Apple\napple"
        );

        // Unique keeps first occurrence's original casing/spacing.
        assert_eq!(
            sort_unique_trim(
                "Apple\napple\nAPPLE",
                &SortUniqueTrimOptions {
                    sort: None,
                    unique: true,
                    trim: true,
                    case_sensitive: false,
                }
            )
            .unwrap(),
            "Apple"
        );

        // Descending sort.
        assert_eq!(
            sort_unique_trim(
                "a\nc\nb",
                &SortUniqueTrimOptions {
                    sort: Some(SortOrder::Descending),
                    unique: false,
                    trim: true,
                    case_sensitive: false,
                }
            )
            .unwrap(),
            "c\nb\na"
        );

        // Trailing newline: no phantom blank line.
        assert_eq!(
            sort_unique_trim("a\nb\n", &SortUniqueTrimOptions::default()).unwrap(),
            "a\nb"
        );

        // Mixed line endings normalize before splitting.
        assert_eq!(
            sort_unique_trim("a\r\nb\r\n", &SortUniqueTrimOptions::default()).unwrap(),
            "a\nb"
        );
    }

    #[test]
    fn sort_unique_trim_is_idempotent() {
        let opts = SortUniqueTrimOptions {
            sort: Some(SortOrder::Ascending),
            unique: true,
            trim: true,
            case_sensitive: false,
        };
        let input = "banana\napple\n Banana \napple\ncherry";
        let once = sort_unique_trim(input, &opts).unwrap();
        let twice = sort_unique_trim(&once, &opts).unwrap();
        assert_eq!(once, twice);
    }

    // -- 4. Base64 ------------------------------------------------------------

    #[test]
    fn base64_golden_cases() {
        assert_eq!(
            base64_transform("hello", &Base64Options::default()).unwrap(),
            "aGVsbG8="
        );
        assert_eq!(base64_transform("", &Base64Options::default()).unwrap(), "");
        assert_eq!(
            base64_transform(
                "",
                &Base64Options {
                    mode: Mode::Decode,
                    ..Base64Options::default()
                }
            )
            .unwrap(),
            ""
        );
        assert_eq!(
            base64_transform(
                "aGVsbG8=",
                &Base64Options {
                    mode: Mode::Decode,
                    ..Base64Options::default()
                }
            )
            .unwrap(),
            "hello"
        );
        // Decode accepts unpadded input too.
        assert_eq!(
            base64_transform(
                "aGVsbG8",
                &Base64Options {
                    mode: Mode::Decode,
                    ..Base64Options::default()
                }
            )
            .unwrap(),
            "hello"
        );
        // Padding off on encode.
        assert_eq!(
            base64_transform(
                "hello",
                &Base64Options {
                    padding: false,
                    ..Base64Options::default()
                }
            )
            .unwrap(),
            "aGVsbG8"
        );
        // URL-safe alphabet round trip for bytes that would use +/ in standard.
        let opts = Base64Options {
            alphabet: Base64Alphabet::UrlSafe,
            ..Base64Options::default()
        };
        let encoded = base64_transform("\u{fb}\u{ff}\u{fe}", &opts).unwrap();
        assert!(!encoded.contains('+') && !encoded.contains('/'));
        // Unicode round trips through UTF-8 bytes.
        assert_eq!(
            base64_transform(
                &base64_transform("héllo wörld", &Base64Options::default()).unwrap(),
                &Base64Options {
                    mode: Mode::Decode,
                    ..Base64Options::default()
                }
            )
            .unwrap(),
            "héllo wörld"
        );
    }

    #[test]
    fn base64_decode_invalid_character_errors() {
        let err = base64_transform(
            "abc!",
            &Base64Options {
                mode: Mode::Decode,
                ..Base64Options::default()
            },
        )
        .unwrap_err();
        match err {
            TransformError::InvalidBase64(msg) => assert!(msg.contains('!')),
            other => panic!("expected InvalidBase64, got {other:?}"),
        }
    }

    #[test]
    fn base64_decode_bad_length_errors() {
        let err = base64_transform(
            "abcde",
            &Base64Options {
                mode: Mode::Decode,
                ..Base64Options::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            err,
            TransformError::InvalidBase64(
                "input length is not a multiple of 4 (and no valid unpadded length)".to_string()
            )
        );
    }

    #[test]
    fn base64_decode_non_utf8_errors() {
        // 0xFF 0xFE is not valid UTF-8 in any interpretation.
        let raw_encoded = base64_encode_bytes(&[0xFF, 0xFE], Base64Alphabet::Standard, true);
        let err = base64_transform(
            &raw_encoded,
            &Base64Options {
                mode: Mode::Decode,
                ..Base64Options::default()
            },
        )
        .unwrap_err();
        assert_eq!(err, TransformError::DecodedBytesNotUtf8);
    }

    #[test]
    fn base64_rejects_oversized_input() {
        let huge = "a".repeat(MAX_INPUT_BYTES + 1);
        let err = base64_transform(&huge, &Base64Options::default()).unwrap_err();
        assert!(matches!(err, TransformError::InputTooLarge { .. }));
    }

    proptest! {
        #[test]
        fn base64_roundtrip(s in ".*") {
            let encoded = base64_transform(&s, &Base64Options::default()).unwrap();
            let decoded = base64_transform(
                &encoded,
                &Base64Options { mode: Mode::Decode, ..Base64Options::default() },
            )
            .unwrap();
            prop_assert_eq!(decoded, s);
        }

        #[test]
        fn base64_roundtrip_url_safe_no_padding(s in ".*") {
            let opts = Base64Options {
                mode: Mode::Encode,
                alphabet: Base64Alphabet::UrlSafe,
                padding: false,
            };
            let encoded = base64_transform(&s, &opts).unwrap();
            let decoded = base64_transform(
                &encoded,
                &Base64Options { mode: Mode::Decode, ..opts },
            )
            .unwrap();
            prop_assert_eq!(decoded, s);
        }
    }

    // -- 5. URL encode / decode -----------------------------------------------

    #[test]
    fn url_golden_cases() {
        assert_eq!(
            url_transform("a b&c", &UrlOptions::default()).unwrap(),
            "a%20b%26c"
        );
        assert_eq!(
            url_transform(
                "https://x.com/a b",
                &UrlOptions {
                    mode: Mode::Encode,
                    scope: UrlScope::Full
                }
            )
            .unwrap(),
            "https://x.com/a%20b"
        );
        assert_eq!(url_transform("", &UrlOptions::default()).unwrap(), "");
        assert_eq!(
            url_transform(
                "",
                &UrlOptions {
                    mode: Mode::Decode,
                    ..UrlOptions::default()
                }
            )
            .unwrap(),
            ""
        );
        assert_eq!(
            url_transform(
                "a%20b%26c",
                &UrlOptions {
                    mode: Mode::Decode,
                    ..UrlOptions::default()
                }
            )
            .unwrap(),
            "a b&c"
        );
        // Multi-byte UTF-8 is percent-encoded byte-by-byte.
        assert_eq!(
            url_transform("é", &UrlOptions::default()).unwrap(),
            "%C3%A9"
        );
        // Structural chars stay untouched in Full scope.
        assert_eq!(
            url_transform(
                "a:b/c?d#e[f]@g",
                &UrlOptions {
                    mode: Mode::Encode,
                    scope: UrlScope::Full
                }
            )
            .unwrap(),
            "a:b/c?d#e[f]@g"
        );
    }

    #[test]
    fn url_decode_truncated_percent_errors() {
        let err = url_transform(
            "abc%2",
            &UrlOptions {
                mode: Mode::Decode,
                ..UrlOptions::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, TransformError::InvalidPercentEncoding(_)));
    }

    #[test]
    fn url_decode_invalid_hex_errors() {
        let err = url_transform(
            "abc%zz",
            &UrlOptions {
                mode: Mode::Decode,
                ..UrlOptions::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, TransformError::InvalidPercentEncoding(_)));
    }

    #[test]
    fn url_decode_non_utf8_errors() {
        let err = url_transform(
            "%FF%FE",
            &UrlOptions {
                mode: Mode::Decode,
                ..UrlOptions::default()
            },
        )
        .unwrap_err();
        assert_eq!(err, TransformError::DecodedBytesNotUtf8);
    }

    #[test]
    fn url_rejects_oversized_input() {
        let huge = "a".repeat(MAX_INPUT_BYTES + 1);
        let err = url_transform(&huge, &UrlOptions::default()).unwrap_err();
        assert!(matches!(err, TransformError::InputTooLarge { .. }));
    }

    proptest! {
        #[test]
        fn url_roundtrip_component(s in ".*") {
            let encoded = url_transform(&s, &UrlOptions::default()).unwrap();
            let decoded = url_transform(
                &encoded,
                &UrlOptions { mode: Mode::Decode, ..UrlOptions::default() },
            )
            .unwrap();
            prop_assert_eq!(decoded, s);
        }
    }

    // -- 6. HTML entities -------------------------------------------------------

    #[test]
    fn html_entities_golden_cases() {
        assert_eq!(
            html_entities(
                "<a href=\"x\">it's & 'that'</a>",
                &HtmlEntityOptions::default()
            )
            .unwrap(),
            "&lt;a href=&quot;x&quot;&gt;it&#39;s &amp; &#39;that&#39;&lt;/a&gt;"
        );
        assert_eq!(
            html_entities("", &HtmlEntityOptions::default()).unwrap(),
            ""
        );
        assert_eq!(
            html_entities("&lt;b&gt;", &HtmlEntityOptions { mode: Mode::Decode }).unwrap(),
            "<b>"
        );
        // Numeric entities (decimal and hex) decode.
        assert_eq!(
            html_entities("&#39;&#x27;", &HtmlEntityOptions { mode: Mode::Decode }).unwrap(),
            "''"
        );
        // Unrecognized named entity passes through as literal text.
        assert_eq!(
            html_entities("a&nbsp;b", &HtmlEntityOptions { mode: Mode::Decode }).unwrap(),
            "a&nbsp;b"
        );
        // Unicode outside the 5 reserved chars is untouched both ways.
        assert_eq!(
            html_entities("café <3", &HtmlEntityOptions::default()).unwrap(),
            "café &lt;3"
        );
        assert_eq!(
            html_entities("café &lt;3", &HtmlEntityOptions { mode: Mode::Decode }).unwrap(),
            "café <3"
        );
    }

    proptest! {
        #[test]
        fn html_entities_roundtrip(s in ".*") {
            let encoded = html_entities(&s, &HtmlEntityOptions::default()).unwrap();
            let decoded = html_entities(&encoded, &HtmlEntityOptions { mode: Mode::Decode }).unwrap();
            prop_assert_eq!(decoded, s);
        }
    }

    // -- 7. Hash ----------------------------------------------------------------

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hash(
                "",
                &HashOptions {
                    algorithm: HashAlgorithm::Sha256,
                    case: HexCase::Lower
                }
            )
            .unwrap_err(),
            TransformError::EmptyInput
        );
        assert_eq!(
            hash(
                "abc",
                &HashOptions {
                    algorithm: HashAlgorithm::Sha256,
                    case: HexCase::Lower
                }
            )
            .unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash(
                "abc",
                &HashOptions {
                    algorithm: HashAlgorithm::Sha256,
                    case: HexCase::Upper
                }
            )
            .unwrap(),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
        assert_eq!(
            hash(
                "hello",
                &HashOptions {
                    algorithm: HashAlgorithm::Sha256,
                    case: HexCase::Lower
                }
            )
            .unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn md5_known_vectors() {
        // RFC 1321 test vectors.
        assert_eq!(
            hash(
                "a",
                &HashOptions {
                    algorithm: HashAlgorithm::Md5,
                    case: HexCase::Lower
                }
            )
            .unwrap(),
            "0cc175b9c0f1b6a831c399e269772661"
        );
        assert_eq!(
            hash(
                "abc",
                &HashOptions {
                    algorithm: HashAlgorithm::Md5,
                    case: HexCase::Lower
                }
            )
            .unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hash(
                "abc",
                &HashOptions {
                    algorithm: HashAlgorithm::Md5,
                    case: HexCase::Upper
                }
            )
            .unwrap(),
            "900150983CD24FB0D6963F7D28E17F72"
        );
        assert_eq!(
            hash(
                "message digest",
                &HashOptions {
                    algorithm: HashAlgorithm::Md5,
                    case: HexCase::Lower
                }
            )
            .unwrap(),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }

    #[test]
    fn hash_empty_input_errors_for_both_algorithms() {
        for algorithm in [HashAlgorithm::Sha256, HashAlgorithm::Md5] {
            let err = hash(
                "",
                &HashOptions {
                    algorithm,
                    case: HexCase::Lower,
                },
            )
            .unwrap_err();
            assert_eq!(err, TransformError::EmptyInput);
        }
    }

    // -- 8. QR (stub) -------------------------------------------------------

    #[test]
    fn qr_is_not_implemented() {
        assert!(matches!(
            qr("payload"),
            Err(TransformError::NotImplemented(_))
        ));
    }

    #[test]
    fn qr_empty_input_errors() {
        assert_eq!(qr("").unwrap_err(), TransformError::EmptyInput);
    }

    #[test]
    fn qr_oversized_payload_errors() {
        let huge = "a".repeat(MAX_QR_PAYLOAD_BYTES + 1);
        assert_eq!(
            qr(&huge).unwrap_err(),
            TransformError::QrPayloadTooLarge {
                len: huge.len(),
                max: MAX_QR_PAYLOAD_BYTES
            }
        );
    }

    // -- 9. Count -------------------------------------------------------------

    #[test]
    fn count_golden_cases() {
        assert_eq!(
            count("").unwrap(),
            "Characters: 0 (0 bytes)\nWords: 0\nLines: 0\nBytes: 0"
        );
        assert_eq!(
            count("hello world").unwrap(),
            "Characters: 11 (11 bytes)\nWords: 2\nLines: 1\nBytes: 11"
        );
        // Unicode: char count vs byte count diverge.
        assert_eq!(
            count("café").unwrap(),
            "Characters: 4 (5 bytes)\nWords: 1\nLines: 1\nBytes: 5"
        );
        // Trailing newline does not add a phantom line.
        assert_eq!(
            count("a\nb\n").unwrap(),
            "Characters: 4 (4 bytes)\nWords: 2\nLines: 2\nBytes: 4"
        );
        assert_eq!(
            count("a\nb").unwrap(),
            "Characters: 3 (3 bytes)\nWords: 2\nLines: 2\nBytes: 3"
        );
    }

    #[test]
    fn count_is_well_defined_on_its_own_output() {
        // Semantically odd, but must never panic.
        let once = count("hello world").unwrap();
        let twice = count(&once).unwrap();
        assert!(!twice.is_empty());
    }

    #[test]
    fn count_rejects_oversized_input() {
        let huge = "a".repeat(MAX_INPUT_BYTES + 1);
        let err = count(&huge).unwrap_err();
        assert!(matches!(err, TransformError::InputTooLarge { .. }));
    }

    // -- TransformError::Display ------------------------------------------------

    #[test]
    fn transform_error_display_is_non_empty() {
        let errors = [
            TransformError::EmptyInput,
            TransformError::InvalidBase64("x".to_string()),
            TransformError::InvalidPercentEncoding("x".to_string()),
            TransformError::DecodedBytesNotUtf8,
            TransformError::InputTooLarge { len: 1, max: 2 },
            TransformError::QrPayloadTooLarge { len: 1, max: 2 },
            TransformError::NotImplemented("x"),
        ];
        for err in errors {
            assert!(!err.to_string().is_empty());
        }
    }
}
