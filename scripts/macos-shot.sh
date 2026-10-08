#!/usr/bin/env bash
# Photograph the Mac app over a demo store (specs/009-focus-macos T004).
#
#   scripts/macos-shot.sh <name> [--seed small] [--light|--dark|--both]
#                          [--size 1440x900] [--wait 6] [--no-build]
#
# Builds the FFI with its `demo` feature and bundles the app (ad hoc: an
# in-memory demo reads no Keychain), launches it over the seed with
# `POSTIO_DEMO`, `POSTIO_WINDOW_SIZE` and `POSTIO_APPEARANCE`, waits for the
# window, captures it with `screencapture -l`, and quits it. Pictures land in
# `Design/review/focus-macos/<name>-<appearance>.png` -- untracked, beside the
# design they are compared with (FR-061).
#
# Needs Screen Recording for the terminal, once. Never touches the store on
# disk or the network: the demo is in memory, and its mail is invented.
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
cd "$TREE"

name=""
seed="small"
appearances=(light dark)
size="1440x900"
wait_for=6
build=1
while [ $# -gt 0 ]; do
    case "$1" in
        --seed) seed="$2"; shift 2 ;;
        --light) appearances=(light); shift ;;
        --dark) appearances=(dark); shift ;;
        --both) appearances=(light dark); shift ;;
        --size) size="$2"; shift 2 ;;
        --wait) wait_for="$2"; shift 2 ;;
        --no-build) build=0; shift ;;
        -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *) name="$1"; shift ;;
    esac
done
[ -n "$name" ] || { echo "usage: scripts/macos-shot.sh <name> [options]" >&2; exit 2; }

if [ "$build" = 1 ]; then
    POSTIO_FFI_FEATURES=demo scripts/macos-build.sh >/dev/null
    scripts/macos-bundle.sh >/dev/null
fi
APP="$TREE/macos/build/Postio.app/Contents/MacOS/Postio"
[ -x "$APP" ] || { echo "no bundle at $APP" >&2; exit 1; }

# The design lives in the main checkout's untracked Design/, beside which the
# pictures go; a worktree has no Design/ of its own.
MAIN=$(git worktree list --porcelain | awk '/^worktree /{print $2; exit}')
OUT="$MAIN/Design/review/focus-macos"
mkdir -p "$OUT"

for appearance in "${appearances[@]}"; do
    POSTIO_DEMO="$seed" POSTIO_WINDOW_SIZE="$size" POSTIO_APPEARANCE="$appearance" \
        "$APP" >/dev/null 2>&1 &
    pid=$!
    window=""
    for _ in $(seq 1 40); do
        sleep 0.25
        window=$(swift scripts/macos-window-id.swift "$pid" 2>/dev/null || true)
        [ -n "$window" ] && break
    done
    if [ -z "$window" ]; then
        kill "$pid" 2>/dev/null || true
        echo "the app's window never appeared" >&2
        exit 1
    fi
    # The first page lands, the counts arrive, the heading settles.
    sleep "$wait_for"
    file="$OUT/$name-$appearance.png"
    screencapture -o -x -l"$window" "$file"
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "$file"
done
