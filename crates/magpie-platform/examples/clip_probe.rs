#[cfg(target_os = "macos")]
fn main() {
    use magpie_platform::os::macos::MacClipboard;
    use magpie_platform::Clipboard;
    let mut c = MacClipboard::new().unwrap();
    let s = c.snapshot();
    println!(
        "token={} concealed={} some={}",
        s.change_token,
        s.concealed,
        s.content.is_some(),
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
