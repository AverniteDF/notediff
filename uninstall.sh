#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
  CARGO_BIN="$CARGO_INSTALL_ROOT/bin/notediff"
else
  CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/notediff"
fi
APPLICATIONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
ICON_NAME="io.github.AverniteDF.Notediff"

removed=0
remove_file() {
  if [[ -f "$1" ]]; then
    rm -- "$1"
    removed=1
  fi
}

remove_file "$APPLICATIONS_DIR/io.github.AverniteDF.Notediff.desktop"
remove_file "$ICONS_DIR/scalable/apps/$ICON_NAME.svg"
remove_file "$ICONS_DIR/128x128/apps/$ICON_NAME.png"
remove_file "$CARGO_BIN"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICONS_DIR" || true
fi
if command -v update-desktop-database >/dev/null 2>&1 &&
  [[ -d "$APPLICATIONS_DIR" ]]; then
  update-desktop-database "$APPLICATIONS_DIR" || true
fi

if [[ "$removed" -eq 0 ]]; then
  echo "notediff did not appear to be installed in this location."
else
  echo "Removed notediff. Snapshots and settings under \$XDG_DATA_HOME/notediff were left alone."
fi
