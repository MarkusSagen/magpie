# Magpie — Capture Files (not filename-text) + Kind Coverage Tests

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1

## Problem

Copying files in Finder is captured as **text** (the filenames), classified
`text`, not `file`. macOS puts several representations on the pasteboard when you
copy files — the **file URLs** (`public.file-url`) *and* a plain-text fallback
(the filenames). `MacClipboard::snapshot()` calls `get_text()` first, gets the
filename text, and never looks for file URLs. `Content::Files`/`Kind::File`/the
`prepare` path already exist and are correct — only the macOS *reader* is missing.

## Fix

In `MacClipboard::snapshot()`, **check the pasteboard for file URLs before text**:
- `NSPasteboard::generalPasteboard().pasteboardItems()` → for each
  `NSPasteboardItem`, `stringForType("public.file-url")` → a percent-encoded
  `file://…` URL.
- Convert each to a filesystem path with a **pure** `file_url_to_path(url) ->
  Option<String>` (strip `file://`, percent-decode; non-`file:` → `None`).
- If any paths result → `Content::Files(paths)`; else fall through to text/image
  exactly as today.
- Add the `NSPasteboardItem` feature to `objc2-app-kit`.

Concealed/change-token logic is unchanged. `prepare(Content::Files)` already sets
`kind = File`, `full_text = paths.join("\n")`, metrics — so the entry shows the
📁 glyph, filters under **File**, and previews the filenames.

**Out of scope (noted follow-up):** pasting a File entry back as real file URLs
(today `set_content(Files)` writes the paths as text). Capture + classification is
this change.

## Tests

**Pure (macOS, runs here):** `file_url_to_path`:
- `file:///Users/me/a%20b.png` → `/Users/me/a b.png`
- `file:///tmp/x` → `/tmp/x`
- `https://example.com` → `None`; `""` → `None`.

**Core kind coverage (new `crates/magpie-core/tests/kinds.rs`)** — ingest each
`Content` and assert the stored `Entry.kind` + that a kind-filtered search returns
it and excludes the others:
- `Content::Text("hello")` → `text`
- `Content::Text("https://x.io/a")` → `link`
- `Content::Text("me@x.io")` → `email`
- `Content::Text("#1a2b3c")` / `"rgb(1,2,3)"` → `color`
- `Content::Image { bytes }` → `image`, `image_path` set, `full_text` empty
- `Content::Files(vec![...])` → `file`, `full_text` = joined paths
- A `SearchQuery { kind: Some(Kind::File), .. }` returns only the file entry, etc.

**App glyph placement:** `type_glyph` already has `every_kind_has_a_glyph`; extend
nothing unless a gap appears. (Confirms each kind → the right row icon.)

## Files
- Modify: `crates/magpie-platform/src/os/macos.rs` (`file_url_to_path` + `file_urls`
  + snapshot order) + `#[cfg(test)]` unit test; `crates/magpie-platform/Cargo.toml`
  (`NSPasteboardItem` feature).
- Create: `crates/magpie-core/tests/kinds.rs`.

## Risks
- Some non-Finder apps may also expose `public.file-url`; treating that as a file
  copy is correct/acceptable. If an app puts a file-url *and* means text, files win
  — matches Finder/Raycast behavior.
- `pasteboardItems()`/`stringForType` are `unsafe` objc2 calls; wrap tightly, return
  `None` on any miss so capture always degrades to text.
