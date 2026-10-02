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
#                                [--calibration]   # play only the reviewer's calibration set
#   scripts/storyboards.sh lint                    # load and lint the whole catalogue
#   scripts/storyboards.sh page  [--open]          # Design/review/<branch>/index.html from the runs
#   scripts/storyboards.sh key   [--app classic|focus|all]   # the review key for HEAD's tree
#   scripts/storyboards.sh bundle --acceptance <file> [--calibration]   # what a reviewer reads
#   scripts/storyboards.sh tool  <postio-storyboard arguments>          # the pure tool, built
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

usage() { sed -n '2,37p' "$0" | sed 's/^# \{0,1\}//'; }

COMMAND="${1:-}"
[ -n "$COMMAND" ] || { usage; exit 2; }
shift
# `tool` hands everything after it to postio-storyboard untouched.
TOOL_ARGS=()
if [ "$COMMAND" = tool ]; then
    TOOL_ARGS=("$@")
    set --
fi

APP=classic
ONLY=""
OPEN=0
CALIBRATION=0
ACCEPTANCE=""
RUNNER_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --app)       APP="${2:?--app needs classic, focus or all}"; shift 2 ;;
        --only)      ONLY="${2:?--only needs a pattern}"; shift 2 ;;
        --no-frames) RUNNER_ARGS+=(--no-frames); shift ;;
        --delivery)  RUNNER_ARGS+=(--delivery "${2:?--delivery needs chain or direct}"); shift 2 ;;
        --open)      OPEN=1; shift ;;
        --calibration) CALIBRATION=1; shift ;;
        --acceptance) ACCEPTANCE="${2:?--acceptance needs a file}"; shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *) echo "storyboards.sh: unknown argument '$1' -- try --help" >&2; exit 2 ;;
    esac
done

BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)
REVIEW="$ROOT/Design/review/${BRANCH//\//-}"
RUNS="$REVIEW/runs"
# The calibration set is the reviewer's test, not the branch's: its runs
# live apart, so a page about the branch never shows them.
BUNDLE="$REVIEW/bundle"
if [ "$CALIBRATION" = 1 ]; then
    RUNS="$REVIEW/calibration/runs"
    BUNDLE="$REVIEW/calibration/bundle"
fi
BASE_RUNS="$REVIEW/base"

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

# Every storyboard a run plays: not calibration, not gap lists -- or, with
# --calibration, only the calibration set the reviewer is measured on.
selected() {
    local file relative
    {
        if [ "$CALIBRATION" = 1 ]; then
            find "$CATALOGUE/calibration" -name '*.toml'
        else
            find "$CATALOGUE" -name '*.toml' -not -path "$CATALOGUE/calibration/*" \
                -not -path "$CATALOGUE/gaps/*"
        fi
    } | sort | while read -r file; do
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
    # The key the runs belong to, so the page's summary can name it and a
    # landing can tell it is current (research R13).
    KEY=$(APP="$APP" review_key 2>/dev/null | tail -1)
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
            --tree-key "$KEY" --commit "$(git rev-parse HEAD 2>/dev/null)" \
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

# The bundle a reviewer reads (contracts/review.md): the runs, the base if
# there is one, the acceptance, and the design screens the storyboards name
# -- from the committed references only. The untracked Focus design folder
# carries a real name and is never offered; the tool refuses it besides.
bundle_command() {
    local bin base=()
    [ -n "$ACCEPTANCE" ] || { echo "storyboards.sh: bundle needs --acceptance <file>" >&2; exit 2; }
    bin=$(tool)
    if [ -d "$BASE_RUNS" ] && [ "$CALIBRATION" = 0 ]; then
        base=(--base "$BASE_RUNS")
    fi
    rm -rf "$BUNDLE"
    "$bin" bundle --runs "$RUNS" ${base[@]+"${base[@]}"} --acceptance "$ACCEPTANCE" \
        --catalogue "$CATALOGUE" --design-dir "$ROOT/Design/screens" --out "$BUNDLE"
    exit $?
}

page_command() {
    local bin review=()
    bin=$(tool)
    mkdir -p "$REVIEW"
    if [ -d "$BUNDLE" ]; then
        review=(--bundle "$BUNDLE")
    fi
    local out="$REVIEW/index.html"
    [ "$CALIBRATION" = 0 ] || out="$REVIEW/calibration/index.html"
    local key
    key=$(review_key 2>/dev/null | tail -1)
    "$bin" page --runs "$RUNS" --prefix "$(realpath --relative-to="$(dirname "$out")" "$RUNS")" \
        --out "$out" --title "$BRANCH" --key "$key" ${review[@]+"${review[@]}"} || exit 2
    if [ "$OPEN" = 1 ]; then
        xdg-open "$out" >/dev/null 2>&1 &
    fi
}

# The review key (research R13): the git tree ids of what a review depends
# on -- the app's crates and the catalogue -- at HEAD. A rebase that does not
# touch them keeps the key; a commit sha would not survive one. A crate not
# on this branch is keyed as absent, so adding it changes the key.
review_key() {
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

key_command() {
    review_key
}

case "$COMMAND" in
    run)  run_command ;;
    bundle) bundle_command ;;
    tool) bin=$(tool); "$bin" ${TOOL_ARGS[@]+"${TOOL_ARGS[@]}"}; exit $? ;;
    key)  key_command ;;
    lint) lint_command ;;
    page) page_command ;;
    -h|--help) usage ;;
    *) echo "storyboards.sh: unknown command '$COMMAND' -- try --help" >&2; exit 2 ;;
esac
