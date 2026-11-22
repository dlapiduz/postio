#!/usr/bin/env bash
# What a diff obliges CI to build, as `key=value` lines for $GITHUB_OUTPUT.
#
#     git diff --name-only base...head | scripts/ci-changes.sh pull_request
#     rust=yes
#     docs=no
#     macos=yes
#
# Gated at job level so every check still reports (#1107, #1127). Fails safe: `rust=yes` unless
# every changed path is known not to reach a compile; an empty list, an unknown path and an event
# without a diff build everything. `macos` is a different question (#666): the Swift half and its
# link need a runner when either half changes, but only `postio-ffi`'s dependency closure can change
# the bindings (#1449). `scripts/lib/ffi-closure.py` computes the outside set each run.
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

# Paths that cannot reach a Rust compile; everything else is Rust-shaped, including `crates/**`
# fixtures.
# `macos/` and the scripts that build and bundle the application are picked up by `MACOS` instead
# (#666).
NOT_RUST='^(docs/|Design/|\.claude/|macos/|README\.md$|CLAUDE\.md$|[^/]*\.md$|\.gitmessage$|\.gitignore$|LICENSE|mise\.toml$|\.github/workflows/(hooks|pages|audit|bench|fuzz|mutants|nightly|release|full-suite)\.yml$|scripts/(tests/|checks/|macos-[a-z]*\.sh$|ffi-bindgen\.sh$|issue-[a-z-]*\.sh$|test-(fast|sanity|headless|with-flake-retry)\.sh$|wait-for-checks\.sh$|full-suite-crates\.sh$|ci-(changes|tooling-needed|frontends-gate)\.sh|risk-of-diff\.sh$|coverage\.sh$|coverage-floors\.json$|check\.sh$|cross-check\.sh$|fuzz(-seed)?\.sh$|mutants\.sh$|release-bump\.py$|report-advisory-failure\.sh$|run-isolated\.sh$|install-local\.sh$|lib/((ready-labels|require-gh)\.sh|ffi-closure\.py)$))'
DOCS='^(docs/|README\.md$|\.github/workflows/ci\.yml$)'
# What obliges the macOS runner beyond a Rust build: `macos/**` except prose, and the scripts that
# build, test and bundle the app.
MACOS='^(macos/(Sources|Tests|Resources)/|macos/Package\.swift$|macos/\.gitignore$|scripts/(macos-[a-z]*|ffi-bindgen)\.sh$|scripts/tests/test-ffi-bindgen\.py$)'
# What is Rust-shaped by name; an unknown path builds everything.
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

# A Rust build obliges macOS too, unless every Rust-shaped path is outside `postio-ffi`'s dependency
# closure (#1449).
# Fails safe: no helper answer, a `crates/<dir>/` not in the set, or a non-crate path (root
# manifest, `.cargo/`, toolchain pin, build script) keeps obliging macOS.
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
