#!/usr/bin/env bash
# Run the tooling self-tests and show the output of the ones that failed.
#
# This lived inline in `ci.yml`, and it decided which logs to print by
# grepping their *content*:
#
#     if ! grep -qE "passed|ok\b" "$log"; then ... tail -n 40 "$log"
#
# Every self-test prints `ok  <case>` per passing case before reporting its
# failures, so that filter skipped the log of any test that failed after
# getting one case right -- which is nearly all of them. The job announced
#
#     ::error::a tooling self-test failed; its output follows
#
# and then followed with nothing. It happened twice in one day (#1243, #1254),
# and both times the only route to the cause was reproducing it locally; the
# second was a real race in a fixture, not a flake. A step that says "here is
# why" and then does not is worse than one that says nothing, because it is
# the one you believe.
#
# Which tests failed is known from their exit status. Nothing needs guessing
# from their text.
#
# A script and not fifteen lines of YAML for the reason `ci-tooling-needed.sh`
# is one: an inline workflow step cannot be tested, which is a poor property
# for the step that keeps the tooling honest.
# `scripts/tests/test-run-self-tests.py` is what tests it.
#
# Usage:
#   scripts/run-self-tests.sh [--jobs N] [--dir DIR] [--logs DIR]
#
# Exit status: 0 every self-test that applies here passed, 1 one or more
# failed, and a test that stood down for this platform (exit 77) is counted
# and named rather than folded into either.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIR="$HERE/tests"
# `-P4` and not more: several of these build a cargo sandbox, and
# .cargo/config.toml already pins two jobs per cargo.
JOBS=4
LOGS=""

while [ $# -gt 0 ]; do
    case "$1" in
        --jobs) JOBS="$2"; shift 2 ;;
        --dir)  DIR="$2"; shift 2 ;;
        --logs) LOGS="$2"; shift 2 ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

[ -d "$DIR" ] || { echo "no such directory: $DIR" >&2; exit 2; }
if [ -z "$LOGS" ]; then
    LOGS="$(mktemp -d)"
    trap 'rm -rf "$LOGS"' EXIT
fi
mkdir -p "$LOGS"

LIST="$LOGS/.self-tests"
# Sorted, so a failure is reported in the same order twice.
find "$DIR" -maxdepth 1 -name 'test-*.py' | sort > "$LIST"
COUNT="$(wc -l < "$LIST" | tr -d ' ')"
[ "$COUNT" -gt 0 ] || { echo "no self-tests under $DIR" >&2; exit 2; }

echo "running $COUNT self-tests, $JOBS at a time"

# One log per test, named after it, and the failures recorded by *exit status*
# rather than by what they printed.
#
# `< "$LIST"` and not `-a "$LIST"`: `-a` is a GNU extension and BSD xargs
# rejects the option outright. With stderr going to /dev/null below, that
# failure was invisible -- no child ever ran, `.failed` stayed empty, and this
# script printed "every self-test passed" and exited 0 on every Mac. Which
# hid `test-run-self-tests.py`, the one test that reports this script broken,
# along with all eighty-four others. #1151 is the issue about self-tests only
# ever running on Linux; this is that issue inside its own runner, and it is
# the same GNU-ism that cost `jobserver.sh` months of two-job builds.
#
# A child's exit status says which of three things happened: 0 passed,
# 77 stood down, anything else failed.
#
# Exit 77 is the third: a self-test declaring that it cannot be
# meaningful on this platform (`scripts/lib/prereq.py`'s `only_on`). #1151
# asks that such a test *say so* rather than skip silently, so it is counted
# and named below instead of being folded into the passes -- a Linux-only
# test on a macOS runner has to be visible in the log, or the suite is
# claiming coverage it does not have.
NOT_APPLICABLE_EXIT=77
FAILED="$LOGS/.failed"
STOOD_DOWN="$LOGS/.not-applicable"
: > "$FAILED"
: > "$STOOD_DOWN"
# shellcheck disable=SC2016
xargs -P "$JOBS" -I{} sh -c '
    log="$2/$(basename "$1").log"
    python3 "$1" > "$log" 2>&1
    status=$?
    [ "$status" -eq 0 ] && exit 0
    if [ "$status" -eq '"$NOT_APPLICABLE_EXIT"' ]; then
        printf "%s\n" "$1" >> "$2/.not-applicable"
        exit 0
    fi
    printf "%s\n" "$1" >> "$2/.failed"
    exit 1
' _ {} "$LOGS" < "$LIST" >/dev/null 2>&1

# Success is a positive fact, not the absence of a recorded failure.
#
# The check above was `[ ! -s "$FAILED" ]` alone, which is true both when
# every test passed and when nothing ran at all -- and those are the two
# answers a suite must never confuse. A runner that cannot start its children
# has to say so, or it is the silent skip this whole repository writes gates
# against, in the one place where it hides every other gate.
RAN="$(find "$LOGS" -maxdepth 1 -name '*.py.log' | wc -l | tr -d ' ')"
if [ "$RAN" -ne "$COUNT" ]; then
    echo
    echo "::error::ran $RAN of $COUNT self-tests; the runner could not start them" >&2
    echo "This is not a test failure -- no test reported one. Nothing ran." >&2
    exit 1
fi

STOOD="$(wc -l < "$STOOD_DOWN" | tr -d ' ')"
if [ "$STOOD" -gt 0 ]; then
    echo
    echo "$STOOD not applicable on this platform, and said so:"
    sed 's|.*/|  |' "$STOOD_DOWN"
fi

if [ ! -s "$FAILED" ]; then
    echo "every self-test that applies here passed"
    exit 0
fi

echo
echo "::error::a tooling self-test failed; its output follows"
while read -r path; do
    [ -n "$path" ] || continue
    name="$(basename "$path")"
    echo
    echo "--- $name ---"
    # Enough to carry a traceback and the case list around it. The old inline
    # version used 40; a failure that scrolls off is the same problem again.
    tail -n 60 "$LOGS/$name.log" 2>/dev/null || echo "(no log was written)"
done < "$FAILED"

echo
echo "failed: $(tr '\n' ' ' < "$FAILED")"
exit 1
