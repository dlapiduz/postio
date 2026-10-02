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
#                                [--variants]      # every variant each storyboard asks for
#                                [--calibration]   # play only the reviewer's calibration set
#   scripts/storyboards.sh lint                    # load and lint the whole catalogue
#   scripts/storyboards.sh page  [--open]          # Design/review/<branch>/index.html from the runs
#   scripts/storyboards.sh key   [--app classic|focus|all]   # the review key for HEAD's tree
#   scripts/storyboards.sh base  [--app classic|focus]   # the branch's storyboards on the merge-base's code
#   scripts/storyboards.sh coverage [--app classic]      # every command in every context: does it show?
#   scripts/storyboards.sh screens [--only <glob>]       # the screen sweep: design beside app, every screen
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

usage() { sed -n '2,41p' "$0" | sed 's/^# \{0,1\}//'; }

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
        --variants)  RUNNER_ARGS+=(--variants); shift ;;
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

# A compositor of this invocation's own. The shared one hosts every other
# session's test windows too, and a window that is not the active one is drawn
# in GTK's backdrop style -- faded buttons, grey rows -- so a frame came out
# differently depending on who else was running. Large enough for the widest
# variant, and stopped on the way out.
HEADLESS_STARTED=0
headless() {
    export POSTIO_TEST_DISPLAY="postio-storyboard-$$"
    export POSTIO_TEST_GEOMETRY="1920x1200"
    if [ "$HEADLESS_STARTED" = 0 ]; then
        HEADLESS_STARTED=1
        trap 'scripts/test-headless.sh --stop >/dev/null 2>&1 || true' EXIT
    fi
    scripts/test-headless.sh "$@"
}

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
        headless "$bin" run "${files[@]}" --out "$RUNS" \
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
    local key compare=()
    key=$(review_key 2>/dev/null | tail -1)
    if [ -d "$BASE_RUNS" ] && [ "$CALIBRATION" = 0 ]; then
        compare=(--base "$BASE_RUNS" --base-prefix "$(realpath -s --relative-to="$(dirname "$out")" "$BASE_RUNS")")
    fi
    "$bin" page --runs "$RUNS" --prefix "$(realpath --relative-to="$(dirname "$out")" "$RUNS")" \
        --out "$out" --title "$BRANCH" --key "$key" --catalogue "$CATALOGUE" ${review[@]+"${review[@]}"} \
        ${compare[@]+"${compare[@]}"} || exit 2
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

# The branch's storyboards, played against the base's code (research R8):
# that is what makes "changed" mean behaviour changed, and a storyboard
# written for a defect this branch fixes is red here -- its proof. The base
# tree is a detached worktree inside this tree's target/, so it belongs to
# whoever owns this tree; its build output is a reflink copy of this tree's,
# never a path into it (#1101). Runs are cached by base commit, and a
# storyboard is played again only when its file changed.
base_command() {
    local based sha tree cache crate bin file name hash marker files=() app
    app="${APP:-classic}"
    [ "$app" != all ] || { echo "storyboards.sh: base takes one app at a time" >&2; exit 2; }
    crate=$(runner_crate "$app")
    based=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
    sha=$(git merge-base HEAD "origin/$based" 2>/dev/null) \
        || { echo "storyboards.sh: no merge-base with origin/$based -- fetch first" >&2; exit 2; }
    tree="$ROOT/target/storyboard-base/tree"
    if [ -d "$tree" ]; then
        git -C "$tree" checkout -q --detach "$sha" || exit 2
    else
        mkdir -p "$(dirname "$tree")"
        git worktree add -q --detach "$tree" "$sha" || exit 2
    fi
    if [ ! -f "$tree/crates/$crate/examples/storyboard.rs" ]; then
        echo "base: ${sha:0:12} predates the runner; every storyboard is new against it."
        exit 0
    fi
    cache="${STORYBOARDS_CACHE:-$HOME/.cache/postio/storyboards}/$sha"
    mkdir -p "$cache/.sources"
    while read -r file; do
        name=$(basename "$file" .toml)
        hash=$(sha256sum "$file" | cut -d' ' -f1)
        marker="$cache/.sources/$app-$name"
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
    echo "building the $app runner at ${sha:0:12}..."
    if ! (cd "$tree" && cargo build -q -p "$crate" --example storyboard --features demo) 2>&1 | tail -20 >&2; then
        echo "storyboards.sh: the base runner did not build" >&2
        exit 2
    fi
    bin="$(cd "$tree" && target_dir)/debug/examples/storyboard"
    headless "$bin" run "${files[@]}" --out "$cache" --commit "$sha" \
        ${RUNNER_ARGS[@]+"${RUNNER_ARGS[@]}"}
    # A base run that fails is what a base run of a fixed defect does: it is
    # recorded, not a failure of this command.
    for file in "${files[@]}"; do
        sha256sum "$file" | cut -d' ' -f1 > "$cache/.sources/$app-$(basename "$file" .toml)"
    done
    echo "base: $cache"
}

# The generated pass (spec US6): every command bound in every context,
# pressed from a fresh window, judged on whether a person could see anything
# change. The gap list says which are known to show nothing yet.
coverage_command() {
    local crate bin
    crate=$(runner_crate "$APP")
    echo "building the $APP runner..."
    if ! cargo build -q -p "$crate" --example storyboard --features demo 2>&1 | tail -20 >&2; then
        echo "storyboards.sh: the $APP runner did not build" >&2
        exit 2
    fi
    bin="$(target_dir)/debug/examples/storyboard"
    headless "$bin" every-command --out "$REVIEW/coverage" --gaps "$CATALOGUE/gaps/$APP.toml"
}

# The screen sweep (FR-030), which scripts/screens.sh used to be: every
# zero-step storyboard under storyboards/screens/ in the variants it asks
# for, then the contact sheet -- the canvas design on the left, the app on
# the right. Non-zero, naming them, if any screen failed to render.
screens_command() {
    local crate bin files=() file out code
    crate=$(runner_crate classic)
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
    echo "building the classic runner..."
    if ! cargo build -q -p "$crate" --example storyboard --features demo 2>&1 | tail -20 >&2; then
        echo "storyboards.sh: the runner did not build" >&2
        exit 2
    fi
    bin="$(target_dir)/debug/examples/storyboard"
    headless "$bin" run "${files[@]}" --out "$out/runs" --variants
    bin=$(tool)
    "$bin" sheet --runs "$out/runs" --catalogue "$CATALOGUE" --design-dir "$ROOT/Design/screens" \
        --out "$out/index.html"
    code=$?
    [ "$OPEN" = 0 ] || xdg-open "$out/index.html" >/dev/null 2>&1 &
    exit "$code"
}

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
