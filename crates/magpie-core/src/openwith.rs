//! Categorize a file path so the app can route "open" to a preferred application
//! per category (text/code/image/pdf/ebook/…). Pure + tested; the actual `open -a`
//! lives in the app.

/// A coarse file category, keyed off the extension.
pub fn category_for_path(path: &str) -> &'static str {
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "heic" | "svg"
        | "ico" | "avif" => "image",
        // PDF
        "pdf" => "pdf",
        // Ebooks
        "epub" | "mobi" | "azw" | "azw3" | "fb2" | "cbz" | "cbr" | "djvu" => "ebook",
        // Audio
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "aiff" => "audio",
        // Video
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" => "video",
        // Archives
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "archive",
        // Code / config (open in an editor)
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "h" | "cpp" | "hpp" | "cc"
        | "java" | "kt" | "swift" | "rb" | "php" | "sh" | "zsh" | "bash" | "fish" | "lua"
        | "vim" | "el" | "clj" | "hs" | "ml" | "sql" | "toml" | "yaml" | "yml" | "json" | "xml"
        | "html" | "css" | "scss" | "ini" | "conf" | "cfg" | "dockerfile" | "zig" | "nix"
        | "gradle" | "cmake" | "makefile" => "code",
        // Plain text / docs
        "txt" | "md" | "markdown" | "org" | "rst" | "log" | "csv" | "tsv" | "rtf" | "tex" => "text",
        _ => "other",
    }
}

/// Whether a clip's text looks like a single filesystem path (absolute or `~`).
pub fn looks_like_path(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && !t.contains('\n')
        && (t.starts_with('/') || t.starts_with("~/"))
        && !t.contains("://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(category_for_path("/a/b/photo.PNG"), "image");
        assert_eq!(category_for_path("~/doc.pdf"), "pdf");
        assert_eq!(category_for_path("book.epub"), "ebook");
        assert_eq!(category_for_path("/src/main.rs"), "code");
        assert_eq!(category_for_path("notes.md"), "text");
        assert_eq!(category_for_path("/tmp/archive.tar.gz"), "archive");
        assert_eq!(category_for_path("song.mp3"), "audio");
        assert_eq!(category_for_path("/x/movie.mkv"), "video");
        assert_eq!(category_for_path("/x/unknownfile"), "other");
        assert_eq!(category_for_path("/x/data.bin"), "other");
    }

    #[test]
    fn path_detection() {
        assert!(looks_like_path("/Users/me/file.pdf"));
        assert!(looks_like_path("~/Downloads/a.png"));
        assert!(!looks_like_path("https://example.com/a.png"));
        assert!(!looks_like_path("just some text"));
        assert!(!looks_like_path("line one\nline two"));
    }
}
