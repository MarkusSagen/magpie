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
    echo "built $APP (unsigned)"

# Windows: the release .exe is the deliverable
package-windows: release
    @echo "deliverable: target/release/magpie.exe"

# Linux: binary + autostart .desktop template
package-linux: release
    @echo "deliverable: target/release/magpie (+ ~/.config/autostart via --install-autostart)"
