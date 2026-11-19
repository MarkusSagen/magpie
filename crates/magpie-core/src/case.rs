//! Case conversion (Phase 13): split an identifier/phrase into words and re-join
//! them in one of nine styles. The tokenizer is acronym-aware, so `HTTPServer`
//! and `parseJSONValue` split the way a programmer expects.

/// Split `input` into lowercase word tokens. Boundaries: whitespace, `_`, `-`,
/// `.`, `/`, and camelCase transitions. Acronyms stay whole until the last
/// capital that starts the next word: `HTTPServer` → `["http", "server"]`,
/// `parseJSONValue` → `["parse", "json", "value"]`, `getID` → `["get", "id"]`.
pub fn tokenize(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, words: &mut Vec<String>| {
        if !cur.is_empty() {
            words.push(std::mem::take(cur).to_lowercase());
        }
    };
    for i in 0..chars.len() {
        let c = chars[i];
        if c.is_whitespace() || matches!(c, '_' | '-' | '.' | '/' | ':' | ',') {
            flush(&mut cur, &mut words);
            continue;
        }
        if !c.is_alphanumeric() {
            // Drop other punctuation but treat it as a boundary.
            flush(&mut cur, &mut words);
            continue;
        }
        if !cur.is_empty() {
            let prev = chars[i - 1];
            // lower/digit → Upper starts a new word (fooBar, a1B).
            let lower_to_upper = (prev.is_lowercase() || prev.is_ascii_digit()) && c.is_uppercase();
            // UPPER followed by Upper+lower ends an acronym (HTTPServer → HTTP|Server):
            // split before `c` when `c` is the last capital of a run that precedes a lowercase.
            let acronym_end = prev.is_uppercase()
                && c.is_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_lowercase();
            if lower_to_upper || acronym_end {
                flush(&mut cur, &mut words);
            }
        }
        cur.push(c);
    }
    flush(&mut cur, &mut words);
    words
}

fn cap_first(w: &str) -> String {
    let mut cs = w.chars();
    match cs.next() {
        Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
        None => String::new(),
    }
}

/// The nine case styles, as `(style_name, converted_value)`, in the order the
/// answer card lists them (screen 17). Empty input yields empty values.
pub fn all_cases(input: &str) -> Vec<(&'static str, String)> {
    let w = tokenize(input);
    let camel = if w.is_empty() {
        String::new()
    } else {
        w[0].clone() + &w[1..].iter().map(|x| cap_first(x)).collect::<String>()
    };
    let pascal = w.iter().map(|x| cap_first(x)).collect::<String>();
    let snake = w.join("_");
    let kebab = w.join("-");
    let screaming = w.join("_").to_uppercase();
    let train = w.iter().map(|x| cap_first(x)).collect::<Vec<_>>().join("-");
    let title = w.iter().map(|x| cap_first(x)).collect::<Vec<_>>().join(" ");
    let upper = w.join(" ").to_uppercase();
    let lower = w.join(" ");
    vec![
        ("camelCase", camel),
        ("PascalCase", pascal),
        ("snake_case", snake),
        ("kebab-case", kebab),
        ("SCREAMING_SNAKE_CASE", screaming),
        ("Train-Case", train),
        ("Title Case", title),
        ("UPPERCASE", upper),
        ("lowercase", lower),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_camel_snake_kebab_and_acronyms() {
        assert_eq!(tokenize("fooBarBaz"), ["foo", "bar", "baz"]);
        assert_eq!(tokenize("foo_bar-baz qux"), ["foo", "bar", "baz", "qux"]);
        assert_eq!(tokenize("HTTPServer"), ["http", "server"]);
        assert_eq!(tokenize("parseJSONValue"), ["parse", "json", "value"]);
        assert_eq!(tokenize("getID"), ["get", "id"]);
        assert_eq!(tokenize("a1B2"), ["a1", "b2"]);
    }

    #[test]
    fn all_nine_styles() {
        let m: std::collections::HashMap<_, _> =
            all_cases("HTTP server value").into_iter().collect();
        assert_eq!(m["camelCase"], "httpServerValue");
        assert_eq!(m["PascalCase"], "HttpServerValue");
        assert_eq!(m["snake_case"], "http_server_value");
        assert_eq!(m["kebab-case"], "http-server-value");
        assert_eq!(m["SCREAMING_SNAKE_CASE"], "HTTP_SERVER_VALUE");
        assert_eq!(m["Train-Case"], "Http-Server-Value");
        assert_eq!(m["Title Case"], "Http Server Value");
        assert_eq!(m["UPPERCASE"], "HTTP SERVER VALUE");
        assert_eq!(m["lowercase"], "http server value");
    }

    #[test]
    fn empty_input_is_all_empty() {
        for (_, v) in all_cases("   ") {
            assert_eq!(v, "");
        }
    }

    #[test]
    fn single_word_roundtrips() {
        let m: std::collections::HashMap<_, _> = all_cases("hello").into_iter().collect();
        assert_eq!(m["camelCase"], "hello");
        assert_eq!(m["PascalCase"], "Hello");
        assert_eq!(m["snake_case"], "hello");
    }
}
