#[cfg(target_os = "macos")]
fn main() {
    use magpie_platform::os::macos::MacClipboard;
    use magpie_platform::os::source_app::ActiveWinSource;
    use magpie_platform::{Clipboard, SourceApp};
    let mut c = MacClipboard::new().unwrap();
    let s = c.snapshot();
    let app = ActiveWinSource {
        cache_dir: std::env::temp_dir().join("magpie-probe-icons"),
    }
    .frontmost();
    println!(
        "token={} concealed={} some={} app={:?}",
        s.change_token,
        s.concealed,
        s.content.is_some(),
        app.map(|a| a.display_name),
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
