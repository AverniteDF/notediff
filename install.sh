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
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/notediff"
SCRIPTS_DIR="$DATA_DIR/scripts"
THEMES_DIR="$DATA_DIR/themes"
DOCS_DIR="$DATA_DIR/docs"
SCRIPT_STATE_FILE="$DATA_DIR/script-state"
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

if [[ -d "$PROJECT_DIR/examples/scripts" ]]; then
  mkdir -p "$SCRIPTS_DIR"
  for script in "$PROJECT_DIR/examples/scripts"/*; do
    [[ -f "$script" ]] || continue
    dest="$SCRIPTS_DIR/$(basename "$script")"
    if [[ ! -e "$dest" ]]; then
      install -m 755 "$script" "$dest"
    fi
  done
fi

if [[ -d "$PROJECT_DIR/examples/themes" ]]; then
  mkdir -p "$THEMES_DIR"
  for theme_dir in "$PROJECT_DIR/examples/themes"/*; do
    [[ -d "$theme_dir" ]] || continue
    dest="$THEMES_DIR/$(basename "$theme_dir")"
    if [[ ! -e "$dest" ]]; then
      mkdir -p "$dest"
      install -m 644 "$theme_dir/theme.conf" "$theme_dir/scheme.xml" "$dest/"
    fi
  done
fi

if [[ -f "$PROJECT_DIR/docs/scripts.md" ]]; then
  mkdir -p "$DOCS_DIR"
  install -m 644 "$PROJECT_DIR/docs/scripts.md" "$DOCS_DIR/scripts.md"
fi

preset_header_value() {
  local preset_file="$1" target_key="$2"
  TARGET_KEY="$target_key" awk '
    {
      sub(/\r$/, "")
      if ($0 == "---") exit
      if ($0 ~ /^[[:space:]]*$/ || $0 ~ /^[[:space:]]*#/) next
      eq = index($0, "=")
      if (eq == 0) next
      key = substr($0, 1, eq - 1)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
      if (key == ENVIRON["TARGET_KEY"]) {
        val = substr($0, eq + 1)
        sub(/^ /, "", val)
        print val
        exit
      }
    }
  ' "$preset_file"
}

render_preset_block() {
  local preset_file="$1" scripts_dir_real="$2" docs_dir_real="$3"
  SCRIPTS_DIR_REAL="$scripts_dir_real" DOCS_DIR_REAL="$docs_dir_real" awk '
    function escape_line(s,    n, parts, i, out) {
      n = split(s, parts, /\\/)
      out = parts[1]
      for (i = 2; i <= n; i++) out = out "\\\\" parts[i]
      gsub(/\t/, "\\t", out)
      gsub(/\r/, "\\r", out)
      return out
    }
    function replace_token(s, tok, repl,    pos) {
      while ((pos = index(s, tok)) > 0) {
        s = substr(s, 1, pos - 1) repl substr(s, pos + length(tok))
      }
      return s
    }
    BEGIN { in_body = 0; body_lines = 0; extra_count = 0 }
    {
      sub(/\r$/, "")
      if (!in_body) {
        if ($0 == "---") {
          in_body = 1
          next
        }
        if ($0 ~ /^[[:space:]]*$/ || $0 ~ /^[[:space:]]*#/) next
        eq = index($0, "=")
        if (eq == 0) next
        key = substr($0, 1, eq - 1)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
        val = substr($0, eq + 1)
        sub(/^ /, "", val)
        if (key == "script") {
          script = val
        } else if (key == "name") {
          name = val
        } else if (key == "body_field") {
          body_field = val
        } else {
          if (index(val, "@DOCS_DIR@") > 0) {
            if (ENVIRON["DOCS_DIR_REAL"] == "") exit 2
            val = replace_token(val, "@DOCS_DIR@", ENVIRON["DOCS_DIR_REAL"])
          }
          if (index(val, "@SCRIPTS_DIR@") > 0) {
            val = replace_token(val, "@SCRIPTS_DIR@", ENVIRON["SCRIPTS_DIR_REAL"])
          }
          extra_keys[++extra_count] = key
          extra_vals[extra_count] = val
        }
      } else {
        body[++body_lines] = escape_line($0)
      }
    }
    END {
      if (script == "" || name == "") exit 1
      printf "\n[preset %s/%s]\n", ENVIRON["SCRIPTS_DIR_REAL"], script
      printf "name = %s\n", name
      if (body_field != "" && body_lines > 0) {
        printf "field.%s = ", body_field
        for (i = 1; i <= body_lines; i++) {
          if (i > 1) printf "\\n"
          printf "%s", body[i]
        }
        printf "\n"
      }
      for (i = 1; i <= extra_count; i++) {
        printf "%s = %s\n", extra_keys[i], extra_vals[i]
      }
    }
  ' "$preset_file"
}

has_preset() {
  local state_file="$1" script_path="$2" preset_name="$3"
  [[ -f "$state_file" ]] || return 1
  SCRIPT_HEADER="[preset $script_path]" PRESET_NAME="$preset_name" awk '
    {
      sub(/\r$/, "")
      if ($0 ~ /^\[/) {
        in_target = ($0 == ENVIRON["SCRIPT_HEADER"])
        next
      }
      if (in_target && $0 ~ /^[[:space:]]*name[[:space:]]*=/) {
        val = $0
        sub(/^[[:space:]]*name[[:space:]]*=[ ]?/, "", val)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", val)
        if (val == ENVIRON["PRESET_NAME"]) {
          found = 1
          exit 0
        }
      }
    }
    END { exit(found ? 0 : 1) }
  ' "$state_file"
}

if [[ -d "$PROJECT_DIR/examples/presets" && -d "$SCRIPTS_DIR" ]]; then
  scripts_dir_real="$(cd "$SCRIPTS_DIR" && pwd -P)"
  docs_dir_real=""
  if [[ -d "$DOCS_DIR" ]]; then
    docs_dir_real="$(cd "$DOCS_DIR" && pwd -P)"
  fi
  pending_presets=()
  for preset in "$PROJECT_DIR/examples/presets"/*.preset; do
    [[ -f "$preset" ]] || continue
    preset_script="$(preset_header_value "$preset" script)"
    preset_name="$(preset_header_value "$preset" name)"
    [[ -n "$preset_script" && -n "$preset_name" ]] || continue
    [[ -f "$SCRIPTS_DIR/$preset_script" ]] || continue
    if ! has_preset "$SCRIPT_STATE_FILE" "$scripts_dir_real/$preset_script" "$preset_name"; then
      pending_presets+=("$preset")
    fi
  done

  if (( ${#pending_presets[@]} > 0 )); then
    mkdir -p "$DATA_DIR"
    tmp_state="$(mktemp "$DATA_DIR/.script-state.XXXXXX")"
    if [[ -f "$SCRIPT_STATE_FILE" ]]; then
      cp -p "$SCRIPT_STATE_FILE" "$tmp_state"
      if [[ -s "$tmp_state" && "$(tail -c 1 "$tmp_state" | wc -l)" -eq 0 ]]; then
        printf '\n' >> "$tmp_state"
      fi
    else
      printf '# notediff script state (written by notediff)\n' > "$tmp_state"
      chmod 644 "$tmp_state"
    fi

    added=0
    for preset in "${pending_presets[@]}"; do
      if render_preset_block "$preset" "$scripts_dir_real" "$docs_dir_real" >> "$tmp_state"; then
        added=1
      fi
    done

    if (( added )); then
      mv "$tmp_state" "$SCRIPT_STATE_FILE"
    else
      rm -f "$tmp_state"
    fi
  fi
fi

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
echo "  scripts: $SCRIPTS_DIR"
echo "  themes:  $THEMES_DIR"
