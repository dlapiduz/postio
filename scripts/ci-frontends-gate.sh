#!/usr/bin/env bash
# The verdict of the `Frontends` check: did the GTK suites actually pass?
#
#     scripts/ci-frontends-gate.sh <rust> <changes-result> <job>=<result>...
#     scripts/ci-frontends-gate.sh yes success frontend-build=success test-frontends=success
#
# `rust` is `needs.changes.outputs.rust`, `changes-result` is `needs.changes.result`, each
# `<job>=<result>` a needed job's `needs.<job>.result`.
# One always-running gate gives the ruleset a fixed name: the shards have matrix names, and a
# skipped job counts as passed.
# A skip counts only when `changes` succeeded and said `rust=no`; any other non-success, or an
# unreadable answer, fails.
# `macos` is not one of the jobs: it is gated on a different question and belongs to the Mac port's
# lane.
# Tested by `scripts/tests/test-ci-frontends-gate.py`.
set -uo pipefail

rust=${1:-}; changes=${2:-}
shift 2 2>/dev/null || true

fail() { echo "::error::Frontends: $*"; exit 1; }

[ "$changes" = success ] || fail "the change classification did not succeed ($changes), so a skip proves nothing"
case "$rust" in
    yes|no) ;;
    *) fail "the classification's rust answer is '$rust'; expected yes or no" ;;
esac
[ $# -gt 0 ] || fail "no jobs were given to check"

for entry in "$@"; do
    job=${entry%%=*}; result=${entry#*=}
    case "$result" in
        success) echo "$job: success" ;;
        skipped)
            [ "$rust" = no ] || fail "$job was skipped but Rust changed; it did not run"
            echo "$job: skipped, and no Rust changed" ;;
        failure|cancelled) fail "$job: $result" ;;
        *) fail "$job: unreadable result '$result'" ;;
    esac
done
echo "Frontends: ok (rust=$rust)"
