#!/usr/bin/env bash
# Installs aurwatch end to end:
#   1. the binary        -> ~/.local/bin/aurwatch (shared by CLI, timer and widget)
#   2. the fetch timer   -> ~/.config/systemd/user/aurwatch-fetch.{service,timer}, enabled
#   3. the bar widget    -> ~/.config/omarchy/plugins/custom.aurwatch (LOCAL-ONLY:
#      never `omarchy plugin add`, never the marketplace; same as cyberwatch)
#
#   ./install.sh                 install everything; widget left disabled
#   ./install.sh --enable        ...and add the widget to the bar (right section)
#   ./install.sh --no-timer      skip the systemd timer
#
# Safe to rerun: it replaces the binary and copies over the plugin files.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ID="custom.aurwatch"
PLUGIN_DIR="$HOME/.config/omarchy/plugins/$PLUGIN_ID"
UNIT_DIR="$HOME/.config/systemd/user"

enable_widget=0
timer=1
for arg in "$@"; do
  case "$arg" in
    --enable) enable_widget=1 ;;
    --no-timer) timer=0 ;;
    -h | --help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

step() { printf '\n\033[1m▸ %s\033[0m\n' "$1"; }

step "Building and installing the binary"
cargo install --locked --path "$ROOT" --root "$HOME/.local"
"$HOME/.local/bin/aurwatch" --version

if ((timer)); then
  step "Installing the 6-hourly fetch timer"
  mkdir -p "$UNIT_DIR"
  cp "$ROOT/contrib/systemd/aurwatch-fetch.service" "$ROOT/contrib/systemd/aurwatch-fetch.timer" "$UNIT_DIR/"
  systemctl --user daemon-reload
  systemctl --user enable --now aurwatch-fetch.timer
  systemctl --user list-timers aurwatch-fetch.timer --no-pager | sed -n '1,2p'
fi

step "Installing the bar widget → $PLUGIN_DIR"
# Real copies, not symlinks: `omarchy plugin validate` rejects symlinks.
# Only files whose content or mode differs are copied, and the shell is only
# asked to rescan when something changed: a rescan of every plugin keeps the
# shell busy for a few seconds, timing out the next `omarchy plugin` command.
mkdir -p "$PLUGIN_DIR"
changed=0
place() { # place <mode> <file>
  local mode="$1" src="$ROOT/$2" dst="$PLUGIN_DIR/$2"
  if [[ -f "$dst" ]] && cmp -s "$src" "$dst" && [[ "$(stat -c %a "$dst")" == "$mode" ]]; then
    return 0
  fi
  install -m "$mode" "$src" "$dst"
  echo "  updated $2"
  changed=1
}
place 644 manifest.json
place 644 BarWidget.qml
place 755 aurwatch-toggle
if command -v omarchy >/dev/null 2>&1; then
  omarchy plugin validate "$PLUGIN_DIR"
fi
if ((changed)) && command -v omarchy-shell >/dev/null 2>&1; then
  echo "  rescanning plugins"
  omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
elif ((!changed)); then
  echo "  plugin files unchanged, no rescan needed"
fi

# First data for the widget, if there's none yet.
if ! "$HOME/.local/bin/aurwatch" summary | grep -q '"have_data":true'; then
  step "Fetching for the first time"
  "$HOME/.local/bin/aurwatch" fetch
fi

# Read the bar layout from the shell's config instead of asking the shell,
# which may still be busy with a rescan.
in_bar() {
  local cfg="$HOME/.config/omarchy/shell.json"
  [[ -f "$cfg" ]] && command -v jq >/dev/null 2>&1 &&
    jq -e --arg id "$PLUGIN_ID" '[.bar.layout[]?[]?.id?] | index($id) != null' "$cfg" >/dev/null 2>&1
}

echo
if in_bar; then
  echo "The widget is in the bar. To remove it: omarchy plugin disable $PLUGIN_ID"
elif ((enable_widget)); then
  step "Adding the widget to the bar"
  omarchy plugin enable "$PLUGIN_ID" --section right
else
  echo "Widget installed but not in the bar yet. To add it:"
  echo "  omarchy plugin enable $PLUGIN_ID --section right"
fi
