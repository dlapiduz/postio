#!/usr/bin/env bash
# The landing tier: every crate's unit tests, and nothing that links the world.
#
# Three tiers, and picking the right one is most of what makes iterating here
# cheap or expensive:
#
#   scripts/test-fast.sh    between edits    the crates you changed, --lib
#   scripts/test-sanity.sh  before landing   the whole workspace, --lib
#   scripts/issue-land.sh --full   when you want the integration suites too
#
# # Why this tier exists
#
# Measured on this workstation, warm:
#
#     cargo test --workspace --lib     1,313 tests   4.96 s wall, 12.3 s CPU
#     the full suite                   3,169 tests   ~497 s on CI; postio-app's
#                                                    app_suite alone is an
#                                                    ~11-minute compile and link
#
# 19 binaries against 197. That is not a smaller version of the suite, it is a
# different order of cost, and it is the whole reason this file exists: several
# sessions share this machine with `jobs = 2`, so two gate runs contend and
# landing became something you queued for rather than something you did.
#
# # What it does not prove, which matters more than what it does
#
# **Unit tests are exactly the tier that cannot see this project's
# characteristic bug.** Every layer here is tested and passes; the failures
# that reach users live *between* them. The Reader was built, tested, and
# never mounted -- you could not read mail in a mail client and every test was
# green. The search UI was fed by nothing. #70 happened twice.
#
# So a green run here means the units hold. It does not mean the application
# works, and it is not a licence to stop writing integration tests -- CI still
# runs the whole workspace on every pull request, and that is what makes
# leaning on this one safe. If that ever changes, this comment is wrong and
# should be rewritten rather than quietly left here.
#
# # Why it has no sleeps
#
# Not by policy, by construction: the things that sleep are waiting on a main
# loop, a socket or a compositor, and those live in `tests/`. A unit test with
# a sleep in it is usually a unit test that wanted to be an integration test.
#
# Usage:
#   scripts/test-sanity.sh            # the whole workspace's unit tests
#   scripts/test-sanity.sh -- quote   # pass a filter through to cargo test
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
cd "$TREE"
# The linker and CC in .cargo/config.toml are names on PATH, not paths (#1101).
[ -x scripts/install-shims.sh ] && scripts/install-shims.sh

FILTER=""
while [ $# -gt 0 ]; do
    case "$1" in
        --) shift; FILTER="${1:-}"; break ;;
        -h|--help) sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unexpected argument: $1" >&2; exit 2 ;;
    esac
done

STARTED=$(date +%s)

if [ -n "$FILTER" ]; then
    cargo test --workspace --lib -- "$FILTER"
else
    cargo test --workspace --lib
fi

echo
echo "sanity tier passed in $(( $(date +%s) - STARTED ))s."
echo "It proves the units, not the wiring: scripts/issue-land.sh --full runs"
echo "the integration suites, and CI runs them on every pull request."
