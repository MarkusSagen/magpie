pub struct TextMetrics {
    pub char_count: i64,
    pub word_count: i64,
    pub line_count: i64,
}

pub fn text_metrics(s: &str) -> TextMetrics {
    if s.is_empty() {
        return TextMetrics {
            char_count: 0,
            word_count: 0,
            line_count: 0,
        };
    }
    let char_count = s.chars().count() as i64;
    let word_count = s.split_whitespace().count() as i64;
    let line_count = 1 + s.matches('\n').count() as i64;
    TextMetrics {
        char_count,
        word_count,
        line_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_all_zero() {
        let m = text_metrics("");
        assert_eq!((m.char_count, m.word_count, m.line_count), (0, 0, 0));
    }

    #[test]
    fn single_line_counts() {
        let m = text_metrics("hello world");
        assert_eq!((m.char_count, m.word_count, m.line_count), (11, 2, 1));
    }

    #[test]
    fn multiline_counts_lines_and_words() {
        let m = text_metrics("a b\nc\n");
        // chars: a,space,b,\n,c,\n = 6 ; words: a,b,c = 3 ; lines: 1 + 2 newlines = 3
        assert_eq!((m.char_count, m.word_count, m.line_count), (6, 3, 3));
    }

    #[test]
    fn unicode_scalar_values_not_bytes() {
        let m = text_metrics("café");
        assert_eq!(m.char_count, 4);
    }
}
