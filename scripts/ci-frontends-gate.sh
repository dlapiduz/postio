#!/usr/bin/env bash
# The verdict of the `Frontends` check: did the GTK suites actually pass?
#
#     scripts/ci-frontends-gate.sh <rust> <changes-result> <job>=<result>...
#     scripts/ci-frontends-gate.sh yes success test-gtk=success test-app=success
#
# `rust` is `needs.changes.outputs.rust`, `changes-result` is
# `needs.changes.result`, and each `<job>=<result>` is a needed job's
# `needs.<job>.result`.
#
# # Why this is a gate and not a required check on each job
#
# `Tests (GTK widgets N/2)` and `Tests (application)` are skipped when no Rust
# changed, and GitHub counts a skipped job as passed. They also have matrix
# names, which a ruleset cannot list stably. One job that needs them both and
# always runs gives the ruleset a single fixed name -- and four GTK-only
# failures reached main because nothing required those suites.
#
# # The trap
#
# A gate that treats "skipped" as fine reports green having done nothing: a
# job skipped because `changes` failed, or was cancelled, or classified wrongly
# looks the same as one skipped because nothing needed it. So a skip counts
# only when the classification itself succeeded and said `rust=no`. Anything
# else that is not `success` fails, and so does an answer this script cannot
# read.
#
# `macos` is deliberately not one of the jobs: it is gated on a different
# question (`macos`, not `rust`), runs for fourteen minutes on a runner that is
# not ours, and belongs to the Mac port's own lane. Adding it later is one more
# argument and one more `needs`.
#
# `scripts/tests/test-ci-frontends-gate.py` is what tests it.
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
