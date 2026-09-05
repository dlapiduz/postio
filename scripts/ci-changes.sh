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
# against. So `macos=yes` whenever `rust=yes`, plus for `macos/` and the
# scripts that assemble the application -- and `macos/**` on its own leaves
# `rust=no`, which is the whole saving.
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
NOT_RUST='^(docs/|Design/|\.claude/|macos/|README\.md$|CLAUDE\.md$|[^/]*\.md$|\.gitmessage$|\.gitignore$|LICENSE|mise\.toml$|\.github/workflows/(hooks|pages|audit|bench|fuzz|mutants|nightly|release)\.yml$|scripts/(tests/|checks/|macos-[a-z]*\.sh$|ffi-bindgen\.sh$|issue-[a-z-]*\.sh$|test-(fast|sanity|headless|with-flake-retry)\.sh$|wait-for-checks\.sh$|full-suite-crates\.sh$|ci-(changes|tooling-needed)\.sh$|coverage\.sh$|coverage-floors\.json$|check\.sh$|cross-check\.sh$|fuzz(-seed)?\.sh$|mutants\.sh$|release-bump\.py$|report-advisory-failure\.sh$|run-isolated\.sh$|install-local\.sh$|lib/(ready-labels|require-gh)\.sh$))'
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

# Everything that obliges a Rust build obliges the macOS one too: the
# bindings are generated from the crate on every build, so a boundary change
# reaches the Swift compiler whether or not any Swift changed.
[ "$rust" = yes ] && macos=yes

printf 'rust=%s\ndocs=%s\nmacos=%s\n' "$rust" "$docs" "$macos"
