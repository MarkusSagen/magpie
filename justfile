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
    # Ad-hoc sign with a stable identifier so macOS TCC (Accessibility) can bind the
    # grant to a consistent identity. Without any signature the grant often won't
    # apply even when the toggle is on. (Developer-ID signing/notarization is a
    # separate, later step — see the install spec.)
    codesign --force --deep --sign - --identifier io.magpie "$APP" \
      && echo "built $APP (ad-hoc signed)" \
      || echo "built $APP (unsigned — codesign unavailable)"

# Windows: the release .exe is the deliverable
package-windows: release
    @echo "deliverable: target/release/magpie.exe"

# Linux: binary + autostart .desktop template
package-linux: release
    @echo "deliverable: target/release/magpie (+ ~/.config/autostart via --install-autostart)"
