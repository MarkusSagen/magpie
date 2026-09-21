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

# macOS: assemble a minimal .app bundle around the release binary
package-macos: release
    #!/usr/bin/env bash
    set -euo pipefail
    APP="target/Magpie.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    cp target/release/magpie "$APP/Contents/MacOS/magpie"
    cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
    cp packaging/macos/Magpie.icns "$APP/Contents/Resources/Magpie.icns"
    # Sign so macOS TCC (Accessibility) binds the grant to a consistent identity.
    # Ad-hoc (`-`) changes the code hash every build, so the Accessibility grant
    # RESETS on each rebuild. Set SIGN_ID to a STABLE self-signed cert (see
    # `just make-signing-cert`) and the grant persists across rebuilds.
    SIGN_ID="${SIGN_ID:--}"
    codesign --force --deep --sign "$SIGN_ID" --identifier io.magpie "$APP" \
      && echo "built $APP (signed: $SIGN_ID)" \
      || echo "built $APP (unsigned — codesign unavailable)"
    if [ "$SIGN_ID" = "-" ]; then echo "NOTE: ad-hoc signed — the Accessibility grant resets each rebuild. See 'just make-signing-cert' for a stable grant."; fi

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

# Windows: the release .exe is the deliverable
package-windows: release
    @echo "deliverable: target/release/magpie.exe"

# Linux: binary + autostart .desktop template
package-linux: release
    @echo "deliverable: target/release/magpie (+ ~/.config/autostart via --install-autostart)"
