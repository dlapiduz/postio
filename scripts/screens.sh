#!/usr/bin/env bash
# Render every screen the application can draw, into one directory, beside the
# design it is supposed to match.
#
# Why this exists
# ---------------
# The test suite is good and it is not enough. Every visual defect found in
# #1179 and #1195 -- a settings pane that went blank when an unrelated table
# had a typo, a new window that never followed dark mode, a 1px rule drawn as
# a grey block down the middle of a pane, an account form that hid the list it
# belonged to -- passed every test in the repository and was caught by
# somebody looking at a picture. `shot` renders one screen; this renders all
# of them, so looking is one command rather than thirty.
#
# The output is a contact sheet pairing each rendered screen with the design
# screen it answers to, which is the form a `/ux-architect` review actually
# needs: not "here is the app", but "here is the app next to what it was
# supposed to be".
#
# Usage
# -----
#   scripts/screens.sh                    # everything, into Design/review/<date>
#   scripts/screens.sh --out /tmp/sweep   # somewhere else
#   scripts/screens.sh --only settings    # just the screens whose name matches
#   scripts/screens.sh --list             # names and arguments, render nothing
#
# It exits non-zero if any screen failed to render, and says which -- a sweep
# that half worked and said nothing is how a blank pane gets reviewed as
# though it were a design decision.
set -uo pipefail

cd "$(dirname "$0")/.."

OUT="Design/review/$(date +%Y-%m-%d)"
ONLY=""
LIST=0
while [ $# -gt 0 ]; do
    case "$1" in
        --out)  OUT="${2:?--out needs a directory}"; shift 2 ;;
        --only) ONLY="${2:?--only needs a pattern}"; shift 2 ;;
        --list) LIST=1; shift ;;
        -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "screens.sh: unknown argument '$1' -- try --help" >&2; exit 2 ;;
    esac
done

# name|design screen it answers to (or -)|arguments to `shot`
#
# The design column is the point of the sheet. `-` means the app draws
# something the canvas never drew, which is worth seeing on its own: it is
# either a gap in the drawings or a screen nobody designed.
SCREENS=$(cat <<'TABLE'
inbox|01-inbox-reading|demo
inbox-dark|07-dark|demo dark
inbox-high-contrast|-|demo dark hc
inbox-narrow|-|demo 900x700
density-compact|08-density-compact|demo compact
density-comfortable|-|demo comfortable
row-selected|06-mouse-parity-undo|demo selected
reader|20-reader-view|demo open 1600x900
reader-bulk-mail|20-reader-view|demo open shipping 1600x900
conversation|17-conversation-view|demo thread 1600x900
thread-drill-in|19-threaded-view|demo thread 1400x900
search|05-search|demo search
command-palette|-|demo command
contact-card|-|demo contact
folder-picker|-|demo folder
compose|03-compose|demo compose
compose-detached|-|demo compose detached
first-run|09-first-run|demo orientation
store-locked|12-states-empty-offline-syncfail|locked
syncing|12-states-empty-offline-syncfail|demo syncing
backfill|-|demo backfill
settings-accounts|21-settings-window-accounts|demo settings
settings-account-form|13-settings-accounts|demo settings account
settings-account-weights|11-attachments-mime|demo settings weights
settings-signatures|-|demo settings account signature
settings-appearance|22-settings-panes|demo settings appearance
settings-keyboard|22-settings-panes|demo settings keyboard
settings-storage|22-settings-panes|demo settings storage
settings-privacy|-|demo settings privacy
settings-filters|-|demo settings filters
settings-composing|-|demo settings composing
settings-config-file|10-settings-config|demo settings configfile
settings-dark|-|demo settings appearance dark
add-account|14-add-account|demo addaccount
add-account-browser|15-oauth-flow|demo addaccount browser
add-account-sync-window|16-settings-panes|demo addaccount syncwindow
TABLE
)

if [ "$LIST" = 1 ]; then
    printf '%-26s %-34s %s\n' NAME DESIGN ARGUMENTS
    while IFS='|' read -r name design args; do
        [ -n "$name" ] || continue
        printf '%-26s %-34s %s\n' "$name" "$design" "$args"
    done <<< "$SCREENS"
    exit 0
fi

mkdir -p "$OUT"

# Built once. `cargo run` would re-check freshness before each of thirty-odd
# renders, and a sweep that takes a minute longer than it needs to is a sweep
# people stop running.
echo "building the shot example..."
if ! cargo build -q -p postio-app --example shot 2>&1 | tail -20; then
    echo "screens.sh: the shot example did not build" >&2
    exit 1
fi
SHOT=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/examples/shot
[ -x "$SHOT" ] || { echo "screens.sh: no shot binary at $SHOT" >&2; exit 1; }

failed=0
rendered=0
skipped=0
ROWS=""

while IFS='|' read -r name design args; do
    [ -n "$name" ] || continue
    if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then
        skipped=$((skipped + 1))
        continue
    fi
    printf '  %-26s ' "$name"
    # Word-split deliberately: `args` is a list of mode words, and #599 is
    # what happens when one is passed as a single argument instead.
    # shellcheck disable=SC2086
    if out=$("$SHOT" "$OUT/$name.png" $args 2>&1) && [ -s "$OUT/$name.png" ]; then
        echo "ok   $(echo "$out" | grep -oE '[0-9]+x[0-9]+' | head -1)"
        rendered=$((rendered + 1))
        ROWS="$ROWS$name|$design|ok"$'\n'
    else
        echo "FAILED"
        echo "$out" | sed 's/^/      /' | tail -3
        failed=$((failed + 1))
        ROWS="$ROWS$name|$design|failed"$'\n'
    fi
done <<< "$SCREENS"

# The contact sheet. Design on the left, what the application actually drew on
# the right, one row per screen -- which is the comparison a review is for.
INDEX="$OUT/index.html"
{
    cat <<'HEAD'
<!doctype html><meta charset="utf-8"><title>Postio screens</title>
<style>
 body{font:14px/1.5 system-ui,sans-serif;margin:0;background:#14161a;color:#e6e8eb}
 header{padding:20px 28px;border-bottom:1px solid #2a2e35;position:sticky;top:0;background:#14161a;z-index:1}
 h1{margin:0 0 4px;font-size:17px;letter-spacing:.09em;text-transform:uppercase}
 .meta{color:#98a0ab;font-family:ui-monospace,monospace;font-size:12px}
 section{padding:22px 28px;border-bottom:1px solid #2a2e35}
 h2{margin:0 0 3px;font-size:15px}
 .args{color:#98a0ab;font-family:ui-monospace,monospace;font-size:12px;margin-bottom:12px}
 .pair{display:grid;grid-template-columns:1fr 1fr;gap:18px;align-items:start}
 .pair.one{grid-template-columns:1fr}
 figure{margin:0}
 figcaption{color:#98a0ab;font-family:ui-monospace,monospace;font-size:11px;
            letter-spacing:.08em;text-transform:uppercase;margin-bottom:6px}
 img{max-width:100%;border:1px solid #2a2e35;display:block}
 .missing{color:#98a0ab;font-style:italic;padding:30px;border:1px dashed #2a2e35}
 .failed{color:#ff9c8a}
</style>
HEAD
    echo "<header><h1>Postio screens</h1><div class=meta>"
    echo "$(date '+%Y-%m-%d %H:%M') &middot; $(git rev-parse --short HEAD) &middot;"
    echo "$rendered rendered, $failed failed</div></header>"
    while IFS='|' read -r name design status; do
        [ -n "$name" ] || continue
        args=$(grep "^$name|" <<< "$SCREENS" | cut -d'|' -f3)
        echo "<section><h2>$name</h2><div class=args>shot &mdash; $args</div>"
        if [ "$design" != "-" ] && [ -f "../../screens/$design.png" ]; then
            echo "<div class=pair>"
            echo "<figure><figcaption>designed &mdash; $design</figcaption>"
            echo "<img loading=lazy src=../../screens/$design.png></figure>"
        else
            echo "<div class='pair one'>"
        fi
        if [ "$status" = ok ]; then
            echo "<figure><figcaption>built</figcaption><img loading=lazy src=$name.png></figure>"
        else
            echo "<figure><figcaption class=failed>built &mdash; FAILED TO RENDER</figcaption>"
            echo "<div class='missing failed'>nothing was written. A screen that cannot be"
            echo "rendered is the first thing to look at, not the one to skip.</div></figure>"
        fi
        echo "</div></section>"
    done <<< "$ROWS"
} > "$INDEX"

echo
echo "$rendered rendered, $failed failed${ONLY:+, $skipped skipped by --only}"
echo "contact sheet: $INDEX"
[ "$failed" -eq 0 ] || echo "screens.sh: some screens did not render" >&2
exit $(( failed > 0 ? 1 : 0 ))
