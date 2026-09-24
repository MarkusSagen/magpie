#!/usr/bin/env bash
#
# visual-regression.sh — capture every Magpie surface in BOTH themes to PNGs.
#
# A repeatable visual pass over the whole UI: it launches the debug binary once
# per (theme, surface), asks the window server where the window is, and crops a
# screenshot to it. Diff the output dir against a committed baseline (any image
# tool — `git diff`, ImageMagick `compare`, macOS Preview) to catch layout,
# overflow, icon, and theme regressions that a compile-clean build hides.
#
# macOS only (uses `osascript` + `screencapture`). Steals focus briefly per
# shot, so don't run it while typing. See CLAUDE.md "Seeing the UI".
#
# Usage:
#   scripts/visual-regression.sh [OUT_DIR]        # default: target/screenshots
#   MAGPIE_VR_THEMES="light" scripts/visual-regression.sh   # one theme only
#
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$REPO/target/screenshots}"
BIN="$REPO/target/debug/magpie"
THEMES="${MAGPIE_VR_THEMES:-dark light}"
STEP_MS="${MAGPIE_UI_TOUR_MS:-500}"

# Surfaces to capture: "<tour-step> <filename-stem>". The tour steps map to the
# first-class router targets in runtime::spawn_dev_ui_hooks.
SURFACES=(
  "section:clipboard   clipboard"
  "taskview:today      tasks-today"
  "taskview:list       tasks-list"
  "board               tasks-board"
  "section:notes       notes"
  "noteview:journal    journal"
  "noteview:graph      graph"
  "section:bookmarks   bookmarks"
  "section:stats       stats"
)

if [ ! -x "$BIN" ]; then
  echo "error: $BIN not found — run 'just build' first." >&2
  exit 1
fi

DATA="$(mktemp -d)"
trap 'rm -rf "$DATA"; pkill -x magpie 2>/dev/null || true' EXIT
export MAGPIE_DATA_DIR="$DATA"
export MAGPIE_DB_KEY="$(printf '%064d' 42)"
export MAGPIE_SHOW_ON_LAUNCH=1
export MAGPIE_UI_TOUR_MS="$STEP_MS"
mkdir -p "$OUT"

write_config() { # $1 = true|false for theme_dark
  cat > "$DATA/config.toml" <<EOF
launcher_hotkey = "super+shift+space"
quick_paste_hotkeys = ["super+ctrl+1","super+ctrl+2","super+ctrl+3","super+ctrl+4","super+ctrl+5","super+ctrl+6","super+ctrl+7","super+ctrl+8","super+ctrl+9"]
paste_on_select = true
app_denylist = []
mask_apps = []
mask_patterns = []
mask_visible_chars = 3
fetch_link_favicons = false
fetch_link_previews = false
open_to_today = false
vault_watch = false
log_clock_entries = true
theme_dark = $1
EOF
}

capture() { # $1 = tour step, $2 = output png
  local step="$1" out="$2"
  pkill -x magpie 2>/dev/null || true; sleep 0.3
  MAGPIE_UI_TOUR="$step" "$BIN" >/dev/null 2>&1 &
  sleep 2.0
  local proc; proc=$(pgrep -x magpie | head -1)
  [ -z "$proc" ] && { echo "  $out: no process" >&2; return 1; }
  local i
  for i in 1 2 3 4 5; do
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $proc) to true" 2>/dev/null && break
    sleep 0.4
  done
  sleep 0.6
  local rect
  rect=$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $proc)
      set p to position of window 1
      set z to size of window 1
      return (item 1 of p as string) & \",\" & (item 2 of p as string) & \",\" & (item 1 of z as string) & \",\" & (item 2 of z as string)
    end tell" 2>/dev/null || true)
  [ -z "$rect" ] && { echo "  $out: no rect (retrying once)" >&2; sleep 1.0
    rect=$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $proc)
        set p to position of window 1
        set z to size of window 1
        return (item 1 of p as string) & \",\" & (item 2 of p as string) & \",\" & (item 1 of z as string) & \",\" & (item 2 of z as string)
      end tell" 2>/dev/null || true); }
  [ -z "$rect" ] && { echo "  $out: no rect" >&2; pkill -x magpie 2>/dev/null || true; return 1; }
  screencapture -x -R"$rect" "$out"
  pkill -x magpie 2>/dev/null || true
  echo "  $out  ($(stat -f%z "$out" 2>/dev/null) bytes)"
}

caffeinate -u -t 2 2>/dev/null || true
for theme in $THEMES; do
  case "$theme" in
    dark)  write_config true ;;
    light) write_config false ;;
    *) echo "unknown theme '$theme' (use dark|light)" >&2; exit 1 ;;
  esac
  echo "== $theme =="
  for entry in "${SURFACES[@]}"; do
    read -r step stem <<<"$entry"
    capture "$step" "$OUT/${theme}-${stem}.png" || true
  done
done
echo "done -> $OUT"
