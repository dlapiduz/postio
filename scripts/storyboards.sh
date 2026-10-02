#!/usr/bin/env bash
# Play storyboards against Postio's apps, and build the page that shows them.
#
# Why this exists
# ---------------
# The defects that reach the maintainer are sequence defects -- the keyboard
# left in the search field at launch, Escape that does not leave search, the
# cursor that jumps after an archive -- and a picture of one screen cannot
# show any of them (specs/008-storyboards). A storyboard is an interaction
# written down once, in the command vocabulary every app shares; a runner
# plays it, films every step with the keyboard's region outlined, and
# records where everything was. This is the one command that drives it.
#
# Usage
# -----
#   scripts/storyboards.sh run   [--app classic|focus|all] [--only <glob>] [--no-frames] [--delivery chain|direct]
#   scripts/storyboards.sh lint                    # load and lint the whole catalogue
#   scripts/storyboards.sh page  [--open]          # Design/review/<branch>/index.html from the runs
#   scripts/storyboards.sh key   [--app classic|focus|all]   # the review key for HEAD's tree
#
# `--only` matches a storyboard's path under storyboards/ without `.toml`:
# `--only 'list/*'`, `--only search/escape-leaves-search`. Calibration
# storyboards (the reviewer's own test set) and gap lists are never played
# by `run`.
#
# Runs go to Design/review/<branch>/runs/ (gitignored), the branch's `/`
# turned into `-`. Every window is opened on the private headless
# compositor, never on your display.
#
# Exit status: 0 everything ran and passed; 1 a storyboard failed; 2 a
# storyboard failed to load, a runner could not run, or nothing matched.
# A storyboard that is not applicable or not covered never makes it
# non-zero -- it is counted and shown on the page.
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
CATALOGUE="$ROOT/storyboards"

usage() { sed -n '2,34p' "$0" | sed 's/^# \{0,1\}//'; }

COMMAND="${1:-}"
[ -n "$COMMAND" ] || { usage; exit 2; }
shift

APP=classic
ONLY=""
OPEN=0
RUNNER_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --app)       APP="${2:?--app needs classic, focus or all}"; shift 2 ;;
        --only)      ONLY="${2:?--only needs a pattern}"; shift 2 ;;
        --no-frames) RUNNER_ARGS+=(--no-frames); shift ;;
        --delivery)  RUNNER_ARGS+=(--delivery "${2:?--delivery needs chain or direct}"); shift 2 ;;
        --open)      OPEN=1; shift ;;
        -h|--help)   usage; exit 0 ;;
        *) echo "storyboards.sh: unknown argument '$1' -- try --help" >&2; exit 2 ;;
    esac
done

BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)
REVIEW="$ROOT/Design/review/${BRANCH//\//-}"
RUNS="$REVIEW/runs"

target_dir() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        echo "$CARGO_TARGET_DIR"
    else
        cargo metadata --format-version 1 --no-deps 2>/dev/null \
            | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'
    fi
}

# The pure tool, built once.
tool() {
    if ! cargo build -q -p postio-storyboard 2>&1 | tail -20 >&2; then
        echo "storyboards.sh: postio-storyboard did not build" >&2
        exit 2
    fi
    local bin
    bin="$(target_dir)/debug/postio-storyboard"
    [ -x "$bin" ] || { echo "storyboards.sh: no postio-storyboard at $bin" >&2; exit 2; }
    echo "$bin"
}

# Which crate holds an app's runner. An app whose crate is not on this
# branch (Focus, on main) is reported, not failed.
runner_crate() {
    case "$1" in
        classic) echo postio-app ;;
        focus)   echo postio-focus ;;
        *) echo "storyboards.sh: no runner for app '$1'" >&2; exit 2 ;;
    esac
}

# Every storyboard a run plays: not calibration, not gap lists.
selected() {
    local file relative
    find "$CATALOGUE" -name '*.toml' -not -path "$CATALOGUE/calibration/*" \
        -not -path "$CATALOGUE/gaps/*" | sort | while read -r file; do
        relative="${file#"$CATALOGUE"/}"
        relative="${relative%.toml}"
        # shellcheck disable=SC2053 -- a glob, deliberately
        if [ -z "$ONLY" ] || [[ "$relative" == $ONLY ]]; then
            echo "$file"
        fi
    done
}

run_command() {
    local apps app crate bin status=0 code files
    mapfile -t files < <(selected)
    if [ "${#files[@]}" -eq 0 ]; then
        echo "storyboards.sh: no storyboard matches '${ONLY:-*}'" >&2
        exit 2
    fi
    case "$APP" in
        all) apps="classic focus" ;;
        *)   apps="$APP" ;;
    esac
    mkdir -p "$RUNS"
    for app in $apps; do
        crate=$(runner_crate "$app")
        if [ ! -d "$ROOT/crates/$crate" ]; then
            echo "$app: not present on this branch"
            continue
        fi
        echo "building the $app runner..."
        if ! cargo build -q -p "$crate" --example storyboard --features demo 2>&1 | tail -20 >&2; then
            echo "storyboards.sh: the $app runner did not build" >&2
            exit 2
        fi
        bin="$(target_dir)/debug/examples/storyboard"
        [ -x "$bin" ] || { echo "storyboards.sh: no runner at $bin" >&2; exit 2; }
        # The headless compositor, never the maintainer's display.
        scripts/test-headless.sh "$bin" run "${files[@]}" --out "$RUNS" \
            ${RUNNER_ARGS[@]+"${RUNNER_ARGS[@]}"}
        code=$?
        if [ "$code" -gt "$status" ]; then
            status=$code
        fi
    done
    echo "runs: $RUNS"
    exit "$status"
}

lint_command() {
    local bin
    bin=$(tool)
    "$bin" lint "$CATALOGUE"
    exit $?
}

page_command() {
    local bin
    bin=$(tool)
    mkdir -p "$REVIEW"
    "$bin" page --runs "$RUNS" --prefix runs --out "$REVIEW/index.html" --title "$BRANCH" || exit 2
    if [ "$OPEN" = 1 ]; then
        xdg-open "$REVIEW/index.html" >/dev/null 2>&1 &
    fi
}

# The review key (research R13): the git tree ids of what a review depends
# on -- the app's crates and the catalogue -- at HEAD. A rebase that does not
# touch them keeps the key; a commit sha would not survive one. A crate not
# on this branch is keyed as absent, so adding it changes the key.
key_command() {
    local bin paths path id trees=()
    case "$APP" in
        classic) paths="crates/postio-gtk crates/postio-app crates/postio-ui" ;;
        focus)   paths="crates/postio-focus crates/postio-widgets crates/postio-ui" ;;
        all)     paths="crates/postio-gtk crates/postio-app crates/postio-focus crates/postio-widgets crates/postio-ui" ;;
        *) echo "storyboards.sh: no key for app '$APP'" >&2; exit 2 ;;
    esac
    for path in $paths storyboards; do
        id=$(git rev-parse -q --verify "HEAD:$path" 2>/dev/null || echo absent)
        trees+=(--tree "$path=$id")
    done
    bin=$(tool)
    "$bin" key "${trees[@]}"
}

case "$COMMAND" in
    run)  run_command ;;
    key)  key_command ;;
    lint) lint_command ;;
    page) page_command ;;
    -h|--help) usage ;;
    *) echo "storyboards.sh: unknown command '$COMMAND' -- try --help" >&2; exit 2 ;;
esac
