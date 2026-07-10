#!/usr/bin/env bash
set -euo pipefail

# Resolve the script directory, even when the script is launched from another folder
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(dirname "$SCRIPT_DIR")"

# Always run from the project root
cd "$APP_DIR"

# App configuration
APP_ID="patchwork-app"
APP_NAME="Patchwork"
BIN_NAME="patchwork-app-tauri"

# Source files
ICON_SRC="public/logo.png"

# Install paths
INSTALL_BIN="/usr/local/bin/${APP_ID}"
DESKTOP_FILE="/usr/local/share/applications/${APP_ID}.desktop"
ICON_NAME="${APP_ID}"
ICON_DEST="/usr/local/share/icons/hicolor/256x256/apps/${ICON_NAME}.png"

# Use Cargo's target directory if defined, otherwise use ./target
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"

echo "==> Project root: ${APP_DIR}"

echo "==> Building Leptos frontend in release mode"
cargo leptos build --release --frontend-only

echo "==> Copying index.html into dist/"
cp index.html dist/index.html

echo "==> Building Tauri app in release mode without bundle"
cargo tauri build --no-bundle --ci

# Expected binary path
BIN_SRC="${CARGO_TARGET_DIR}/release/${BIN_NAME}"

# Fallback: search for the binary if the expected path does not exist
if [[ ! -f "$BIN_SRC" ]]; then
    echo "==> Binary not found at: $BIN_SRC"
    echo "==> Searching for ${BIN_NAME} automatically..."

    FOUND_BIN="$(find . "${CARGO_TARGET_DIR}" -path '*/release/*' -type f -executable -name "$BIN_NAME" 2>/dev/null | head -n 1 || true)"

    if [[ -z "$FOUND_BIN" ]]; then
        echo "ERROR: Could not find ${BIN_NAME} inside a release target directory."
        echo
        echo "Available executable files in release directories:"
        find . "${CARGO_TARGET_DIR}" -path '*/release/*' -type f -executable 2>/dev/null || true
        exit 1
    fi

    BIN_SRC="$FOUND_BIN"
fi

# Check icon file
if [[ ! -f "$ICON_SRC" ]]; then
    echo "ERROR: Icon not found at: $ICON_SRC"
    echo
    echo "Available image files:"
    find . -type f \( -name '*.png' -o -name '*.svg' -o -name '*.ico' \) 2>/dev/null || true
    exit 1
fi

echo "==> Installing binary to ${INSTALL_BIN}"
sudo install -Dm755 "$BIN_SRC" "$INSTALL_BIN"

echo "==> Installing icon to ${ICON_DEST}"
sudo install -Dm644 "$ICON_SRC" "$ICON_DEST"

echo "==> Creating desktop launcher at ${DESKTOP_FILE}"
sudo install -d /usr/local/share/applications

printf '%s\n' \
    "[Desktop Entry]" \
    "Type=Application" \
    "Name=${APP_NAME}" \
    "Comment=Patchwork desktop app" \
    "Exec=${INSTALL_BIN}" \
    "Icon=${ICON_NAME}" \
    "Terminal=false" \
    "Categories=Utility;" \
    "StartupNotify=true" | sudo tee "$DESKTOP_FILE" >/dev/null

echo "==> Installing desktop integration tools on Arch Linux"
if command -v pacman >/dev/null 2>&1; then
    sudo pacman -S --needed hicolor-icon-theme desktop-file-utils
fi

echo "==> Updating desktop application database"
if command -v update-desktop-database >/dev/null 2>&1; then
    sudo update-desktop-database /usr/local/share/applications || true
fi

echo "==> Updating icon cache"
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    sudo gtk-update-icon-cache -f -t /usr/local/share/icons/hicolor || true
fi

echo "==> Validating desktop file"
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$DESKTOP_FILE"
fi

echo
echo "Installation completed."
echo
echo "You can run the app from terminal with:"
echo "  ${APP_ID}"
echo
echo "Or open it from your Hyprland launcher, for example:"
echo "  wofi --show drun"
echo "  fuzzel"
echo
echo "Installed files:"
echo "  Binary:  ${INSTALL_BIN}"
echo "  Icon:    ${ICON_DEST}"
echo "  Desktop: ${DESKTOP_FILE}"