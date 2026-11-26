#!/usr/bin/env bash
# Play storyboards against Postio, and build the page that shows them.
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
#   scripts/storyboards.sh run   [--only <glob>] [--no-frames] [--delivery chain|direct]
#                                [--jobs <n>]      # runners side by side (default: half the cores, at most 4)
#                                [--variants]      # every variant each storyboard asks for
#                                [--calibration]   # play only the reviewer's calibration set
#   scripts/storyboards.sh lint                    # load and lint the whole catalogue
#   scripts/storyboards.sh page  [--open]          # Design/review/<branch>/index.html from the runs
#   scripts/storyboards.sh key                     # the review key for HEAD's tree
#   scripts/storyboards.sh base                    # the branch's storyboards on the merge-base's code
#   scripts/storyboards.sh coverage                # every command in every context: does it show?
#   scripts/storyboards.sh screens [--only <glob>]       # the screen sweep: design beside app, every screen
#   scripts/storyboards.sh bundle --acceptance <file> [--calibration]   # what a reviewer reads
#   scripts/storyboards.sh tool  <postio-storyboard arguments>          # the pure tool, built
#
# Storyboards play on Postio, the one desktop app (ADR 0043): its runner is
# postio-gtk's `storyboard` example (specs/007-postio-focus T265), and
# there is no other to choose.
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

usage() { sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; }

COMMAND="${1:-}"
[ -n "$COMMAND" ] || { usage; exit 2; }
shift
TOOL_ARGS=()
if [ "$COMMAND" = tool ]; then
    TOOL_ARGS=("$@")
    set --
fi

ONLY=""""
OPEN=0
CALIBRATION=0
ACCEPTANCE=""
JOBS=""
RUNNER_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --only)      ONLY="${2:?--only needs a pattern}"; shift 2 ;;
        --no-frames) RUNNER_ARGS+=(--no-frames); shift ;;
        --variants)  RUNNER_ARGS+=(--variants); shift ;;
        --delivery)  RUNNER_ARGS+=(--delivery "${2:?--delivery needs chain or direct}"); shift 2 ;;
        --open)      OPEN=1; shift ;;
        --calibration) CALIBRATION=1; shift ;;
        --acceptance) ACCEPTANCE="${2:?--acceptance needs a file}"; shift 2 ;;
        --jobs)      JOBS="${2:?--jobs needs a number}"; shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *) echo "storyboards.sh: unknown argument '$1' -- try --help" >&2; exit 2 ;;
    esac
done

BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)
REVIEW="$ROOT/Design/review/${BRANCH//\//-}"
RUNS="$REVIEW/runs"
BUNDLE="$REVIEW/bundle"
if [ "$CALIBRATION" = 1 ]; then
    RUNS="$REVIEW/calibration/runs"
    BUNDLE="$REVIEW/calibration/bundle"
fi
BASE_RUNS="$REVIEW/base"

# A compositor of this invocation's own: on the shared one a window that is not active is drawn in
# GTK's backdrop style, so frames differed with who else was running. It ends with its runner.
headless() {
    export POSTIO_TEST_DISPLAY="${POSTIO_STORYBOARD_DISPLAY:-postio-storyboard-$$}"
    export POSTIO_TEST_GEOMETRY="1920x1200"
    scripts/test-headless.sh --own "$@"
}

# <path> relative to <dir>, without resolving symlinks; Python, since BSD realpath has no
# `--relative-to`.
relpath() {
    python3 -c 'import os, sys; print(os.path.relpath(sys.argv[1], sys.argv[2]))' "$1" "$2"
}

target_dir() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        echo "$CARGO_TARGET_DIR"
    else
        cargo metadata --format-version 1 --no-deps 2>/dev/null \
            | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'
    fi
}

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

RUNNER_CRATE=postio-gtk
REVIEWED="crates/postio-gtk crates/postio-widgets crates/postio-ui"

build_runner() {
    local dir="$1"
    echo "building the runner..." >&2
    # The attachment text helper too: it is looked for beside the executable, which a runner under
    # examples/ lacks, so every PDF would be skipped (#1824).
    if ! (cd "$dir" && cargo build -q -p "$RUNNER_CRATE" --example storyboard --features demo \
        -p postio-session --bin postio-extract-helper) 2>&1 | tail -20 >&2; then
        echo "storyboards.sh: the runner did not build" >&2
        exit 2
    fi
    local bin
    bin="$(cd "$dir" && target_dir)/debug/examples/storyboard"
    [ -x "$bin" ] || { echo "storyboards.sh: no runner at $bin" >&2; exit 2; }
    echo "$bin"
}

use_helper() {
    local helper
    helper="$(dirname "$(dirname "$1")")/postio-extract-helper"
    [ -x "$helper" ] && export POSTIO_EXTRACT_HELPER="$helper"
    return 0
}

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

jobs() {
    local n="${JOBS:-}"
    if [ -z "$n" ]; then
        n=$(( $(nproc 2>/dev/null || echo 2) / 2 ))
        [ "$n" -le 4 ] || n=4
    fi
    [ "$n" -ge 1 ] 2>/dev/null || n=1
    echo "$n"
}

# Plays <files> on the runner in $(jobs) shards, prints each shard's output in order, and echoes the
# worst exit code last on stdout.
run_shards() {
    local bin="$1"; shift
    local n i code worst=0 commit logs=() pids=() shard
    n=$(jobs)
    [ "$n" -le "$#" ] || n=$#
    commit=$(git rev-parse HEAD 2>/dev/null)
    for ((i = 0; i < n; i++)); do
        shard=()
        local k=0 file
        for file in "$@"; do
            [ $((k % n)) -eq "$i" ] && shard+=("$file")
            k=$((k + 1))
        done
        logs+=("$(mktemp)")
        (
            POSTIO_STORYBOARD_DISPLAY="postio-storyboard-$$-$i"
            headless "$bin" run "${shard[@]}" --out "$RUNS" \
                --tree-key "$KEY" --commit "$commit" \
                ${RUNNER_ARGS[@]+"${RUNNER_ARGS[@]}"}
        ) >"${logs[$i]}" 2>&1 &
        pids+=($!)
    done
    for ((i = 0; i < n; i++)); do
        wait "${pids[$i]}"
        code=$?
        cat "${logs[$i]}" >&2
        rm -f "${logs[$i]}"
        [ "$code" -le "$worst" ] || worst=$code
    done
    echo "$worst"
}

run_command() {
    local bin code files file
    files=()
    # A read loop, not `mapfile`: macOS ships bash 3.2, which has none.
    while IFS= read -r file; do files+=("$file"); done < <(selected)
    if [ "${#files[@]}" -eq 0 ]; then
        echo "storyboards.sh: no storyboard matches '${ONLY:-*}'" >&2
        exit 2
    fi
    mkdir -p "$RUNS"
    KEY=$(review_key 2>/dev/null | tail -1)
    bin=$(build_runner "$ROOT") || exit 2

    use_helper "$bin"
    # The catalogue is split across runners side by side, each on its own headless compositor: every
    # storyboard settles by waiting, so one runner leaves the machine idle.
    code=$(run_shards "$bin" "${files[@]}")
    echo "runs: $RUNS"
    exit "$code"
}

lint_command() {
    local bin
    bin=$(tool)
    "$bin" lint "$CATALOGUE"
    exit $?
}

# The bundle a reviewer reads (contracts/review.md): runs, base, acceptance and the design screens
# the storyboards name, from committed references only;
# the untracked Focus design folder carries a real name and is never offered.
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
    local key compare=()
    key=$(review_key 2>/dev/null | tail -1)
    if [ -d "$BASE_RUNS" ] && [ "$CALIBRATION" = 0 ]; then
        compare=(--base "$BASE_RUNS" --base-prefix "$(relpath "$BASE_RUNS" "$(dirname "$out")")")
    fi
    "$bin" page --runs "$RUNS" --prefix "$(relpath "$RUNS" "$(dirname "$out")")" \
        --out "$out" --title "$BRANCH" --key "$key" --catalogue "$CATALOGUE" ${review[@]+"${review[@]}"} \
        ${compare[@]+"${compare[@]}"} || exit 2
    if [ "$OPEN" = 1 ]; then
        xdg-open "$out" >/dev/null 2>&1 &
    fi
}

# The review key (research R13): the git tree ids of the app's crates and the catalogue at HEAD, so
# a rebase that does not touch them keeps it; a missing crate is keyed as absent.
review_key() {
    local bin path id trees=()
    for path in $REVIEWED storyboards; do
        id=$(git rev-parse -q --verify "HEAD:$path" 2>/dev/null || echo absent)
        trees+=(--tree "$path=$id")
    done
    bin=$(tool)
    "$bin" key "${trees[@]}"
}

key_command() {
    review_key
}

# The branch's storyboards played against the base's code (research R8): a storyboard for a defect
# this branch fixes is red here, its proof.
# The base tree is a detached worktree beside this one, not nested (Cargo merges every ancestor
# `.cargo/config.toml`, #1818), with a reflink copy of this tree's build output (#1101).
# Runs are cached by base commit; a storyboard replays only when its file changed.
base_command() {
    local based sha tree cache bin file name hash marker files=()
    based=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
    sha=$(git merge-base HEAD "origin/$based" 2>/dev/null) \
        || { echo "storyboards.sh: no merge-base with origin/$based -- fetch first" >&2; exit 2; }
    tree="${STORYBOARD_BASE_DIR:-$(dirname "$ROOT")/.storyboard-base}/$(basename "$ROOT")"
    if [ -d "$ROOT/target/storyboard-base/tree" ]; then
        git worktree remove --force "$ROOT/target/storyboard-base/tree" 2>/dev/null \
            || rm -rf "$ROOT/target/storyboard-base/tree"
    fi
    if [ -d "$tree" ]; then
        git -C "$tree" checkout -q --detach "$sha" || exit 2
    else
        mkdir -p "$(dirname "$tree")"
        git worktree add -q --detach "$tree" "$sha" || exit 2
    fi
    if [ ! -f "$tree/crates/$RUNNER_CRATE/examples/storyboard.rs" ]; then
        echo "base: ${sha:0:12} predates the runner; every storyboard is new against it."
        exit 0
    fi
    cache="${STORYBOARDS_CACHE:-$HOME/.cache/postio/storyboards}/$sha"
    mkdir -p "$cache/.sources"
    while read -r file; do
        name=$(basename "$file" .toml)
        hash=$(sha256sum "$file" | cut -d' ' -f1)
        marker="$cache/.sources/$name"
        if [ "$(cat "$marker" 2>/dev/null)" != "$hash" ]; then
            files+=("$file")
        fi
    done < <(selected)
    mkdir -p "$REVIEW"
    ln -sfn "$cache" "$BASE_RUNS"
    if [ "${#files[@]}" -eq 0 ]; then
        echo "base: every storyboard is cached for ${sha:0:12}."
        exit 0
    fi
    if [ ! -d "$tree/target/debug" ] && [ -d "$ROOT/target/debug" ]; then
        mkdir -p "$tree/target"
        cp -a --reflink=auto "$ROOT/target/debug" "$tree/target/" 2>/dev/null || true
    fi
    echo "the base is ${sha:0:12}."
    bin=$(build_runner "$tree") || exit 2

    use_helper "$bin"
    headless "$bin" run "${files[@]}" --out "$cache" --commit "$sha" \
        ${RUNNER_ARGS[@]+"${RUNNER_ARGS[@]}"}
    for file in "${files[@]}"; do
        sha256sum "$file" | cut -d' ' -f1 > "$cache/.sources/$(basename "$file" .toml)"
    done
    echo "base: $cache"
}

# The generated pass (spec US6): every command bound in every context, pressed from a fresh window,
# judged on whether a person could see anything change; the gap list names the known ones.
coverage_command() {
    local bin
    bin=$(build_runner "$ROOT") || exit 2

    use_helper "$bin"
    headless "$bin" every-command --out "$REVIEW/coverage" --gaps "$CATALOGUE/gaps/focus.toml"
}

# The screen sweep (FR-030): every storyboard under storyboards/screens/ filmed on the app, then the
# contact sheet with the canvas design beside it. Non-zero, naming them, if any failed to render.
screens_command() {
    local bin files=() file out code
    while read -r file; do
        files+=("$file")
    done < <(find "$CATALOGUE/screens" -name '*.toml' | sort | while read -r f; do
        rel="${f#"$CATALOGUE"/}"; rel="${rel%.toml}"
        # shellcheck disable=SC2053 -- a glob, deliberately
        if [ -z "$ONLY" ] || [[ "$rel" == $ONLY ]] || [[ "$rel" == screens/$ONLY ]]; then echo "$f"; fi
    done)
    [ "${#files[@]}" -gt 0 ] || { echo "storyboards.sh: no screen matches '${ONLY:-*}'" >&2; exit 2; }
    out="$REVIEW/screens"
    rm -rf "$out/runs"
    mkdir -p "$out/runs"
    bin=$(build_runner "$ROOT") || exit 2

    use_helper "$bin"
    headless "$bin" run "${files[@]}" --out "$out/runs" --variants
    bin=$(tool)
    "$bin" sheet --runs "$out/runs" --catalogue "$CATALOGUE" --design-dir "$ROOT/Design/screens" \
        --out "$out/index.html"
    code=$?
    [ "$OPEN" = 0 ] || xdg-open "$out/index.html" >/dev/null 2>&1 &
    exit "$code"
}

descendants() {
    local child
    for child in $(pgrep -P "$1"); do
        descendants "$child"
        echo "$child"
    done
}

main() {
    case "$COMMAND" in
        run)  run_command ;;
        screens) screens_command ;;
        coverage) coverage_command ;;
        base) base_command ;;
        bundle) bundle_command ;;
        tool) bin=$(tool); "$bin" ${TOOL_ARGS[@]+"${TOOL_ARGS[@]}"}; exit $? ;;
        key)  key_command ;;
        lint) lint_command ;;
        page) page_command ;;
        -h|--help) usage ;;
        *) echo "storyboards.sh: unknown command '$COMMAND' -- try --help" >&2; exit 2 ;;
    esac
}

# Bash defers a trap until the foreground command returns, so the work runs in the background and
# this shell waits: stopped by Ctrl-C or a caller's time cap, it takes its runners with it.
trap 'kill -TERM $(descendants $$) 2>/dev/null; exit 130' INT
trap 'kill -TERM $(descendants $$) 2>/dev/null; exit 143' TERM HUP
main <&0 &
wait $!
