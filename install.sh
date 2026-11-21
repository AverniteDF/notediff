#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
  CARGO_BIN="$CARGO_INSTALL_ROOT/bin/notediff"
else
  CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/notediff"
fi
APPLICATIONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
DESKTOP_NAME="io.github.AverniteDF.Notediff.desktop"
DESKTOP_FILE="$APPLICATIONS_DIR/$DESKTOP_NAME"
ICON_NAME="io.github.AverniteDF.Notediff"
ICON_FILE="$ICONS_DIR/128x128/apps/$ICON_NAME.png"

die() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing '$1'. $2"
}

require_command cargo "Install Rust 1.92 or newer with rustup: https://rustup.rs"
require_command rustc "Install Rust 1.92 or newer with rustup: https://rustup.rs"
require_command pkg-config "Install pkg-config and the GTK development packages."

rust_version="$(rustc --version | awk '{print $2}')"
if [[ "$(printf '%s\n' "1.92" "$rust_version" | sort -V | head -n 1)" != "1.92" ]]; then
  die "Rust $rust_version is too old; notediff needs Rust 1.92 or newer (the GTK bindings need it). Use rustup."
fi

require_pkg_version() {
  local module="$1" minimum="$2" package="$3"
  pkg-config --exists "$module" || die "missing $package development files."
  pkg-config --atleast-version="$minimum" "$module" ||
    die "$package $minimum or newer is required (found $(pkg-config --modversion "$module"))."
}

require_pkg_version gtk4 4.18 "GTK 4"
require_pkg_version libadwaita-1 1.7 "libadwaita"
require_pkg_version gtksourceview-5 5.16 "GtkSourceView 5"

cargo install --locked --path "$PROJECT_DIR" --force
[[ -x "$CARGO_BIN" ]] ||
  die "cargo install completed but $CARGO_BIN was not found; check CARGO_INSTALL_ROOT and CARGO_HOME."

mkdir -p "$APPLICATIONS_DIR"
mkdir -p "$ICONS_DIR/scalable/apps" "$ICONS_DIR/128x128/apps"
install -m 644 "$PROJECT_DIR/assets/$ICON_NAME.svg" \
  "$ICONS_DIR/scalable/apps/$ICON_NAME.svg"
install -m 644 "$PROJECT_DIR/assets/$ICON_NAME.png" \
  "$ICON_FILE"

# Desktop Exec arguments are quoted. Escape backslashes, double quotes,
# backticks, dollar signs and percent signs so an unusual CARGO_HOME still
# produces a valid launcher.
exec_escaped="$(printf '%s' "$CARGO_BIN" |
  sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/`/\\`/g' -e 's/\$/\\$/g' -e 's/%/%%/g')"
NOTEDIFF_EXEC="$exec_escaped" awk '
  /^Exec=/ { print "Exec=\"" ENVIRON["NOTEDIFF_EXEC"] "\" %F"; next }
  { print }
' "$PROJECT_DIR/data/$DESKTOP_NAME" > "$DESKTOP_FILE"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICONS_DIR" || true
fi
if command -v update-desktop-database >/dev/null 2>&1 &&
  [[ -d "$APPLICATIONS_DIR" ]]; then
  update-desktop-database "$APPLICATIONS_DIR" || true
fi

echo "Installed notediff:"
echo "  binary:  $CARGO_BIN"
echo "  desktop: $DESKTOP_FILE"
echo "  icon:    $ICON_FILE"
