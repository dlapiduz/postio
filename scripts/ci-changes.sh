#!/usr/bin/env bash
# What a diff obliges CI to build, as `key=value` lines for $GITHUB_OUTPUT.
#
#     git diff --name-only base...head | scripts/ci-changes.sh pull_request
#     rust=yes
#     docs=no
#     macos=yes
#
# 39% of the last 14 days' commits on main touched no crate, manifest,
# cargo config, toolchain, nextest config, fuzz target or build-affecting
# script -- and unless they were pure prose, CI ran the whole workspace
# suite for them: two tooling PRs waited ~20 minutes each for tests they
# could not affect (#1127). This used to be a workflow-level `paths-ignore`
# for prose only, and it cannot stay one: a workflow that does not run
# reports no check, and a required check that never reports is a pull
# request that never merges (#1107). Gating at job level keeps every check
# reporting -- a skipped job counts as passed -- while skipping the work.
#
# Fails safe. `rust=yes` unless every changed path is one this file knows
# cannot reach a compile; an empty list, an unknown path, and any event
# without a diff to read all build everything. The `docs` key is the same
# shape for the mdbook job.
#
# `macos` is the third question and it is not the same as the first (#666). A
# macOS runner is the only thing that compiles the Swift half and proves the
# link, and it has to run when *either* half changes: Swift cannot reach a
# Rust build, and a Rust change regenerates the bindings the Swift compiles
# against. `macos/**` on its own leaves `rust=no`, which is #666's saving.
#
# It used to be `macos=yes` whenever `rust=yes`, and that was true of the
# reasoning but not of the graph (#1449). What the Swift compiles against is
# generated from `postio-ffi`, so what can change it is `postio-ffi`'s
# dependency closure -- seventeen of the twenty crates. The other three are
# `postio-app`, `postio-gtk` and `postio-bench`, and they carry 78 of the last
# 200 commits on `main`, every one of which started a fourteen-minute job that
# no binding change could have needed. `scripts/lib/ffi-closure.py` computes
# the outside set from the manifests each run; the three names are not written
# down anywhere, here or there, on purpose.
set -euo pipefail

event=${1:-}
case "$event" in
    pull_request|push) ;;
    *) printf 'rust=yes\ndocs=yes\nmacos=yes\n'; exit 0 ;;
esac

files=$(cat)
if [ -z "${files//[[:space:]]/}" ]; then
    printf 'rust=yes\ndocs=yes\nmacos=yes\n'
    exit 0
fi

# Paths that cannot reach a *Rust* compile. Everything else is Rust-shaped,
# including `crates/**` fixtures (tests read them), the manifests, `.cargo/`,
# the toolchain pin, nextest and cargo-deny config, `fuzz/`, the CI workflow
# and action, and the scripts cargo or a build script consults.
#
# `macos/` and the four scripts that build, test and bundle the application
# are here because they cannot change what `cargo test` produces -- they are
# picked up by `MACOS` below instead, which is the saving #666 is after: a
# Swift-only change should not run the whole workspace suite.
NOT_RUST='^(docs/|Design/|\.claude/|macos/|README\.md$|CLAUDE\.md$|[^/]*\.md$|\.gitmessage$|\.gitignore$|LICENSE|mise\.toml$|\.github/workflows/(hooks|pages|audit|bench|fuzz|mutants|nightly|release)\.yml$|scripts/(tests/|checks/|macos-[a-z]*\.sh$|ffi-bindgen\.sh$|issue-[a-z-]*\.sh$|test-(fast|sanity|headless|with-flake-retry)\.sh$|wait-for-checks\.sh$|full-suite-crates\.sh$|ci-(changes|tooling-needed)\.sh$|coverage\.sh$|coverage-floors\.json$|check\.sh$|cross-check\.sh$|fuzz(-seed)?\.sh$|mutants\.sh$|release-bump\.py$|report-advisory-failure\.sh$|run-isolated\.sh$|install-local\.sh$|lib/((ready-labels|require-gh)\.sh|ffi-closure\.py)$))'
DOCS='^(docs/|README\.md$|\.github/workflows/ci\.yml$)'
# What obliges the macOS runner, beyond everything that obliges a Rust build.
# `macos/**` except its prose, and the scripts that build, test and bundle the
# application -- `ffi-bindgen.sh` among them, because the Swift compiles
# against what it writes.
MACOS='^(macos/(Sources|Tests|Resources)/|macos/Package\.swift$|macos/\.gitignore$|scripts/(macos-[a-z]*|ffi-bindgen)\.sh$)'
# What is Rust-shaped by name. A path matching neither list is unknown, and
# unknown builds everything -- the direction that costs minutes, not merges.
RUST='^(crates/|Cargo\.(toml|lock)$|\.cargo/|rust-toolchain\.toml$|\.config/|fuzz/|deny\.toml$|\.github/(workflows/ci\.yml$|actions/)|scripts/)'

rust=no
docs=no
macos=no
while IFS= read -r file; do
    [ -n "$file" ] || continue
    if printf '%s' "$file" | grep -qE "$NOT_RUST"; then
        :
    elif printf '%s' "$file" | grep -qE "$RUST"; then
        rust=yes
    else
        rust=yes; docs=yes
    fi
    printf '%s' "$file" | grep -qE "$DOCS" && docs=yes
    printf '%s' "$file" | grep -qE "$MACOS" && macos=yes
done <<EOF_FILES
$files
EOF_FILES

# Everything that obliges a Rust build obliges the macOS one too -- *unless*
# every Rust-shaped path in the diff is one the bindings cannot be built from.
#
# The bindings the Swift compiles against are generated from `postio-ffi`, so
# what reaches the Swift compiler is `postio-ffi`'s dependency closure, not
# "any Rust". Three crates sit outside it -- and they are the most edited in
# the repository, which is where the fourteen minutes were going (#1449).
#
# `scripts/lib/ffi-closure.py` computes the outside set from the manifests
# every run rather than naming it here; the note at the top of that file says
# why a written-down list is the version that breaks silently.
#
# Fails safe in every direction. If the helper cannot answer, the set is empty
# and every crate obliges macOS. A `crates/<dir>/` path whose directory is not
# in the set -- a new crate, a typo, a rename -- obliges macOS. And only
# `crates/<dir>/` paths are eligible to be excused at all: a root manifest,
# `.cargo/`, the toolchain pin or a build-affecting script has a blast radius
# this cannot bound, so it goes on obliging macOS as before.
if [ "$rust" = yes ] && [ "$macos" = no ]; then
    outside=$("$(dirname "${BASH_SOURCE[0]}")/lib/ffi-closure.py" 2>/dev/null) || outside=""
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        printf '%s' "$file" | grep -qE "$NOT_RUST" && continue
        crate=$(printf '%s' "$file" | sed -n 's|^crates/\([^/]*\)/.*|\1|p')
        if [ -z "$crate" ] || ! printf '%s\n' "$outside" | grep -qxF "$crate"; then
            macos=yes
            break
        fi
    done <<EOF_MACOS
$files
EOF_MACOS
fi

printf 'rust=%s\ndocs=%s\nmacos=%s\n' "$rust" "$docs" "$macos"
