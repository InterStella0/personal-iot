#!/bin/sh
# Builds Home and installs it for the current user: the binary in ~/.local/bin, an app menu
# entry, and an autostart entry that starts it in the tray. Run it again to update.
set -eu
cd "$(dirname "$0")"

app=io.github.interstella0.IotDesktop
bin="$HOME/.local/bin/iot-desktop"

cargo build --release
install -Dm755 target/release/iot-desktop "$bin"

# The portal finds the app by this file's name, so the Talk shortcut needs it installed.
mkdir -p "$HOME/.local/share/applications" "$HOME/.config/autostart"
sed "s|^Exec=iot-desktop$|Exec=$bin|" "$app.desktop" > "$HOME/.local/share/applications/$app.desktop"
sed "s|^Exec=iot-desktop$|Exec=$bin --hidden|" "$app.desktop" > "$HOME/.config/autostart/$app.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "Installed $bin. Start it from the app menu (Home), or: $bin"
