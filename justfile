set shell := ["bash", "-uc"]

# Run the full test suite
test:
    cargo test --workspace

# Run the app (debug)
run:
    cargo run -p magpie-app

# Debug build of everything
build:
    cargo build --workspace

# Optimized release binary
release:
    cargo build --release -p magpie-app

# Print the release binary size
size: release
    ls -lh target/release/magpie* | awk '{print $5, $9}'

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

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
