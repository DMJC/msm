#!/usr/bin/env bash
# Install the release binary, desktop entry and icon for the current user.
# The .desktop file / icon are what Wayland panels use to show the app icon.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release
install -Dm755 target/release/msm "$HOME/.local/bin/msm"
install -Dm644 data/com.matrixmessenger.Msm.desktop "$HOME/.local/share/applications/com.matrixmessenger.Msm.desktop"
install -Dm644 data/icons/hicolor/scalable/apps/com.matrixmessenger.Msm.svg "$HOME/.local/share/icons/hicolor/scalable/apps/com.matrixmessenger.Msm.svg"
gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "Installed. Launch 'Matrix Messenger' from your menu or run msm."
