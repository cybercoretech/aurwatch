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
mkdir -p "$PLUGIN_DIR"
install -m 644 "$ROOT/manifest.json" "$PLUGIN_DIR/manifest.json"
install -m 644 "$ROOT/BarWidget.qml" "$PLUGIN_DIR/BarWidget.qml"
install -m 755 "$ROOT/aurwatch-toggle" "$PLUGIN_DIR/aurwatch-toggle"
if command -v omarchy >/dev/null 2>&1; then
  omarchy plugin validate "$PLUGIN_DIR"
fi
# Let a running shell pick up new or changed plugin files.
if command -v omarchy-shell >/dev/null 2>&1; then
  omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
fi

# First data for the widget, if there's none yet.
if ! "$HOME/.local/bin/aurwatch" summary | grep -q '"have_data":true'; then
  step "Fetching for the first time"
  "$HOME/.local/bin/aurwatch" fetch
fi

if ((enable_widget)); then
  step "Adding the widget to the bar"
  omarchy plugin enable "$PLUGIN_ID" --section right
else
  echo
  echo "Widget installed but not in the bar yet. To add it:"
  echo "  omarchy plugin enable $PLUGIN_ID --section right"
fi
