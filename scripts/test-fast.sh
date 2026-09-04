#!/usr/bin/env bash
# The inner loop: unit tests for the crates you changed, and nothing else.
#
# `issue-land.sh` is what proves work is landable, and it is thorough and slow
# for good reasons. It is the wrong tool to run between one edit and the next,
# and using it that way is most of why iterating here feels expensive: a
# single integration binary in `postio-app` is an eleven-minute compile and
# link, so a red-green-refactor cycle that goes through one costs twenty
# minutes of waiting for maybe two of thinking.
#
# The tests themselves are not the cost. Measured on this workstation:
#
#     postio-body unit tests (49)        0.00s
#     postio-gtk lib tests (330)         0.42s
#     postio-app --test app_suite (43)   11m26s, almost entirely compile+link
#
# So this runs `--lib` only: the unit tests compiled into each crate, which
# link nothing but the crate itself. That is the layer to iterate at, and the
# layer most logic can be made to fail at if it is written as a function
# rather than buried in a widget.
#
# What this does NOT do, deliberately:
#
#   * integration tests (`tests/`), including the app_suite and gtk_suite
#     harnesses -- those link the world, and they are what `issue-land.sh`
#     runs and what proves the layers are joined up;
#   * clippy, formatting, or the repository invariants;
#   * anything at all about crates you did not touch.
#
# **It is not a substitute for landing.** A green run here means the units
# you changed still hold, not that the application works -- the bugs this
# project actually ships live between layers, which is why CLAUDE.md asks for
# assertions on what a person would see. Run it to iterate; land to be sure.
#
# Usage:
#   scripts/test-fast.sh                 # unit tests for the crates you changed
#   scripts/test-fast.sh postio-body     # or for the ones you name
#   scripts/test-fast.sh -- quote        # pass a filter through to cargo test
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
cd "$TREE"
# The linker and CC in .cargo/config.toml are names on PATH, not paths (#1101).
[ -x scripts/install-shims.sh ] && scripts/install-shims.sh

BASE=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)

CRATES=""
FILTER=""
while [ $# -gt 0 ]; do
    case "$1" in
        --) shift; FILTER="${1:-}"; break ;;
        -h|--help) sed -n '2,37p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) CRATES="$CRATES $1"; shift ;;
    esac
done

# No crates named: the ones this branch touches, uncommitted work included.
# Deliberately not fetching first -- this runs between edits, and a network
# round trip in the inner loop is exactly the kind of cost this exists to
# avoid. A stale base only ever widens the list, which is harmless here.
if [ -z "${CRATES// /}" ]; then
    CHANGED=$(git diff --name-only "origin/$BASE...HEAD" 2>/dev/null || true; \
              git status --porcelain | sed 's/^...//')
    CRATES=$(printf '%s\n' $CHANGED | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u)
fi

if [ -z "${CRATES// /}" ]; then
    echo "no crates changed on this branch; nothing to test."
    echo "Name one to run it anyway: scripts/test-fast.sh postio-body"
    exit 0
fi

echo "unit tests only, for:${CRATES}"
echo

STARTED=$(date +%s)
RAN=0
for crate in $CRATES; do
    [ -d "$TREE/crates/$crate" ] || continue
    # A crate with no lib target (postio-bench is nine bench targets and an
    # empty lib; a binary-only crate has nothing to run) is skipped rather
    # than failing the run.
    echo "--- $crate ---"
    if [ -n "$FILTER" ]; then
        cargo test -p "$crate" --lib -- "$FILTER"
    else
        cargo test -p "$crate" --lib
    fi
    RAN=$((RAN + 1))
done

echo
echo "$RAN crate(s) in $(( $(date +%s) - STARTED ))s -- unit tests only."
echo "Integration tests and the invariants run in scripts/issue-land.sh."
