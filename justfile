set shell := ["bash", "-uc"]

# Force the repo's pinned rustup toolchain for EVERY cargo call, defeating this
# machine's three-way toolchain tangle: a `mise` cargo/rustc shim on PATH, an
# ambient `RUSTUP_TOOLCHAIN` (e.g. 1.96) that masks rust-toolchain.toml, and Nix.
# Just using the rustup-proxy cargo isn't enough — the mise `rustc` shim (first on
# PATH) still leaks into proc-macro/host compilation, producing mixed-toolchain
# builds that fail on deps like `convert_case`/`unicode-segmentation` (E0514/E0599).
# So: unset RUSTUP_TOOLCHAIN, put the 1.97.1 bin FIRST on PATH (shadows mise/Nix),
# pin RUSTC, and call the toolchain's cargo directly. Bump the version here when
# rust-toolchain.toml is bumped (and `rustup toolchain install <ver>`).
tc := env_var('HOME') + "/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin"
cargo := "env -u RUSTUP_TOOLCHAIN RUSTC=" + tc + "/rustc PATH=\"" + tc + ":$PATH\" " + tc + "/cargo"

# Run the full test suite
test:
    {{cargo}} test --workspace

# Run the app (debug)
run:
    {{cargo}} run -p magpie-app

# Run the debug build SIGNED with a stable identity so macOS Accessibility
# (auto-paste) actually works in dev. One-time: `just make-signing-cert`, then
# grant `target/debug/magpie` in System Settings ▸ Accessibility once — the grant
# then persists across rebuilds because the signing identity is stable.
#   SIGN_ID defaults to "Magpie Dev" (the cert `just make-signing-cert` creates).
run-signed: build
    #!/usr/bin/env bash
    set -euo pipefail
    SIGN_ID="${SIGN_ID:-Magpie Dev}"
    # Just try to sign — a self-signed cert works with `codesign` even though it is
    # NOT listed by `security find-identity -v -p codesigning` (that flag only shows
    # certs trusted for the whole chain). If it fails, the cert is missing.
    if ! codesign --force --sign "$SIGN_ID" --identifier io.magpie target/debug/magpie 2>/dev/null; then
      echo "Could not sign with identity '$SIGN_ID'. Create it once: just make-signing-cert"; exit 1
    fi
    echo "signed target/debug/magpie as '$SIGN_ID' — grant it once in Accessibility, then auto-paste works."
    ./target/debug/magpie

# Debug build of everything
build:
    {{cargo}} build --workspace

# Optimized release binary
release:
    {{cargo}} build --release -p magpie-app

# Print the release binary size
bin-size: release
    ls -lh target/release/magpie* | awk '{print $5, $9}'

fmt:
    {{cargo}} fmt --all

# Non-mutating format check (for the pre-commit gate / CI).
fmt-check:
    {{cargo}} fmt --all --check

clippy:
    {{cargo}} clippy --workspace --all-targets -- -D warnings

# Show what target/ is using (debug builds are the disk hog — Slint's generated
# code + per-crate artifacts + incremental cache across many test binaries).
size:
    @du -sh target 2>/dev/null || echo "no target/"
    @du -sh target/debug/deps target/debug/incremental target/debug/build 2>/dev/null || true

# Reclaim disk WITHOUT a full recompile: drop only the incremental cache (the
# biggest churn) — the next build reuses compiled deps and just re-links.
slim:
    rm -rf target/debug/incremental target/release/incremental
    @just size

# Reclaim all disk: removes every build artifact; next build is a full recompile.
# With the dev profile stripping dependency debuginfo, a full debug+test build is
# now ~4-5 GB (was ~40 GB before that profile change).
clean:
    {{cargo}} clean

# Install login autostart using the built release binary
install-autostart: release
    ./target/release/magpie --install-autostart

# --- assets ---

# Regenerate every derived icon from the single SVG source in assets/.
# Run this whenever assets/magpie*.svg changes; the outputs are committed so
# `cargo build` and `just package-*` never need a rasterizer (librsvg).
#   - packaging/macos/Magpie.icns        (macOS app icon)
#   - crates/magpie-app/icons/tray-template.png  (menu-bar icon, embedded via include_bytes!)
icons:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v rsvg-convert >/dev/null || { echo "need rsvg-convert (brew install librsvg)"; exit 1; }
    ICONSET="$(mktemp -d)/Magpie.iconset"; mkdir -p "$ICONSET"
    for s in 16 32 128 256 512; do
      rsvg-convert -w "$s"        -h "$s"        assets/magpie.svg -o "$ICONSET/icon_${s}x${s}.png"
      rsvg-convert -w "$((s*2))"  -h "$((s*2))"  assets/magpie.svg -o "$ICONSET/icon_${s}x${s}@2x.png"
    done
    iconutil -c icns "$ICONSET" -o packaging/macos/Magpie.icns
    mkdir -p crates/magpie-app/icons
    rsvg-convert -w 44 -h 44 assets/magpie-mono.svg -o crates/magpie-app/icons/tray-template.png
    echo "regenerated packaging/macos/Magpie.icns + crates/magpie-app/icons/tray-template.png from assets/"

# --- packaging ---
#
# Cross-platform entry points use just's OS attributes (https://just.systems/man/en/
# attributes.html): `just package` / `just install` resolve to the recipe tagged for the
# host OS ([macos]/[linux]/[windows]), like examples/cross-platform.just. The detailed
# macOS recipes (package-macos, dmg, …) stay callable by name.

# Build a distributable package for the current OS (→ target/).
[macos]
[group('package')]
package: package-macos

[linux]
[group('package')]
package: release
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
    STAGE="$(mktemp -d)/magpie-${VERSION}"
    mkdir -p "$STAGE"
    cp target/release/magpie "$STAGE/"
    cp packaging/linux/magpie.desktop "$STAGE/" 2>/dev/null || true
    OUT="target/magpie-${VERSION}-linux-$(uname -m).tar.gz"
    tar -C "$(dirname "$STAGE")" -czf "$OUT" "$(basename "$STAGE")"
    echo "built $OUT"

[windows]
[group('package')]
package: release
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
    OUT="target/magpie-${VERSION}-windows-x86_64.zip"
    # Windows' bundled tar (bsdtar) writes zip with -a; fall back to 7z if present.
    if tar -a -c -f "$OUT" -C target/release magpie.exe 2>/dev/null; then :; \
    elif command -v 7z >/dev/null; then 7z a "$OUT" ./target/release/magpie.exe >/dev/null; \
    else echo "no zip tool found; deliverable: target/release/magpie.exe"; exit 0; fi
    echo "built $OUT"

# Install Magpie for the current OS and register login autostart.
[macos]
[group('install')]
install: install-macos

[linux]
[group('install')]
install: release
    #!/usr/bin/env bash
    set -euo pipefail
    install -Dm755 target/release/magpie "$HOME/.local/bin/magpie"
    install -Dm644 packaging/linux/magpie.desktop "$HOME/.local/share/applications/magpie.desktop" 2>/dev/null || true
    "$HOME/.local/bin/magpie" --install-autostart || true
    echo "installed to ~/.local/bin/magpie (ensure ~/.local/bin is on PATH)"

[windows]
[group('install')]
install: release
    #!/usr/bin/env bash
    echo "Windows: copy target/release/magpie.exe onto your PATH, then run: magpie --install-autostart"

# macOS: assemble a minimal .app bundle around the release binary
[macos]
[group('package')]
package-macos: release
    #!/usr/bin/env bash
    set -euo pipefail
    APP="target/Magpie.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    cp target/release/magpie "$APP/Contents/MacOS/magpie"
    cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
    cp packaging/macos/Magpie.icns "$APP/Contents/Resources/Magpie.icns"
    # Stamp the real crate version into the bundle (the checked-in plist carries a
    # placeholder). plutil edits the copy in place; the source plist is untouched.
    VERSION="$(awk -F'\"' '/^version = /{print $2; exit}' Cargo.toml)"
    plutil -replace CFBundleShortVersionString -string "$VERSION" "$APP/Contents/Info.plist"
    plutil -replace CFBundleVersion            -string "$VERSION" "$APP/Contents/Info.plist"
    # Sign so macOS TCC (Accessibility) binds the grant to a consistent identity.
    # Ad-hoc (`-`) changes the code hash every build, so the Accessibility grant
    # RESETS on each rebuild. Set SIGN_ID to a STABLE self-signed cert (see
    # `just make-signing-cert`) and the grant persists across rebuilds.
    SIGN_ID="${SIGN_ID:--}"
    codesign --force --deep --sign "$SIGN_ID" --identifier io.magpie "$APP" \
      && echo "built $APP (signed: $SIGN_ID)" \
      || echo "built $APP (unsigned — codesign unavailable)"
    if [ "$SIGN_ID" = "-" ]; then echo "NOTE: ad-hoc signed — the Accessibility grant resets each rebuild. See 'just make-signing-cert' for a stable grant."; fi

# Install into /Applications, register login autostart, and launch (SIGN_ID=… for a
# stable Accessibility grant; AUTOSTART=0 to skip the login item).
[macos]
[group('install')]
install-macos: package-macos
    #!/usr/bin/env bash
    set -euo pipefail
    SRC="target/Magpie.app"
    DEST="/Applications/Magpie.app"
    # Quit a running copy so the bundle can be replaced cleanly.
    osascript -e 'tell application "Magpie" to quit' 2>/dev/null || true
    pkill -x magpie 2>/dev/null || true
    sleep 1
    rm -rf "$DEST"
    cp -R "$SRC" "$DEST"
    echo "installed $DEST"
    # Register login autostart (default on; AUTOSTART=0 to skip). Run the INSTALLED
    # binary so the LaunchAgent points at /Applications, not the build tree.
    if [ "${AUTOSTART:-1}" = "1" ]; then
      "$DEST/Contents/MacOS/magpie" --install-autostart && echo "registered login autostart"
    fi
    open "$DEST"
    echo "launched Magpie — look for the menu-bar icon (⌘⇧Space to summon)."

# macOS: build a drag-to-Applications .dmg for distributing to other Macs.
[macos]
[group('package')]
dmg: package-macos
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
    STAGE="$(mktemp -d)/Magpie"
    mkdir -p "$STAGE"
    cp -R target/Magpie.app "$STAGE/Magpie.app"
    ln -s /Applications "$STAGE/Applications"
    OUT="target/Magpie-${VERSION}.dmg"
    rm -f "$OUT"
    hdiutil create -volname "Magpie ${VERSION}" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
    echo "built $OUT (open it, then drag Magpie into Applications)"

# Create a one-time self-signed code-signing cert ("Magpie Dev") so the
# Accessibility grant survives rebuilds. Then build with: SIGN_ID="Magpie Dev" just package-macos
make-signing-cert:
    #!/usr/bin/env bash
    set -euo pipefail
    NAME="Magpie Dev"
    if security find-certificate -c "$NAME" >/dev/null 2>&1; then
      echo "cert '$NAME' already exists — build with: SIGN_ID=\"$NAME\" just package-macos"; exit 0
    fi
    echo "Creating a self-signed code-signing certificate '$NAME' via Keychain Access."
    echo "macOS can't fully script this, so do it once in the GUI:"
    echo "  1. Open Keychain Access ▸ menu Certificate Assistant ▸ Create a Certificate…"
    echo "  2. Name: $NAME   Identity Type: Self Signed Root   Certificate Type: Code Signing"
    echo "  3. Create, then leave it in the 'login' keychain."
    echo "Then rebuild with:  SIGN_ID=\"$NAME\" just package-macos"
    open "/System/Applications/Utilities/Keychain Access.app" || true

