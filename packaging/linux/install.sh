#!/bin/sh
# Install the extracted Linux bundle for the current user, without root privileges.
set -eu

bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"
app_id=com.agentdeck.app

install -Dm755 "$bundle_dir/agentdeck" "$bin_dir/agentdeck"
install -Dm644 "$bundle_dir/logo.png" "$data_dir/icons/hicolor/256x256/apps/$app_id.png"
# Exec is parsed by the desktop shell, not a POSIX shell. Quote its absolute path.
exec_path=$(printf '%s' "$bin_dir/agentdeck" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g; s/%/%%/g')
mkdir -p "$data_dir/applications"
sed "s|^Exec=.*|Exec=/usr/bin/env \"$(printf '%s' "$exec_path" | sed 's/[\\&|]/\\&/g')\"|" \
    "$bundle_dir/$app_id.desktop" > "$data_dir/applications/$app_id.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || :
fi
printf 'AgentDeck installed: %s\n' "$data_dir/applications/$app_id.desktop"
