#!/usr/bin/env bash
# Whether CI needs to run the tooling self-tests for this change.
#
# Prints `yes` or `no`. Reads the changed paths, one per line, on stdin; takes
# the GitHub event name as its only argument.
#
# # Why this is worth a script
#
# `scripts/tests/*.py` is eight of the nine minutes of the `Crate boundaries`
# job, and on a change that cannot affect `scripts/` it buys nothing: every
# self-test builds its own sandbox repository and reads only the copy of the
# tooling it put there. #996 measured that job at 8m24s and called making it
# cheaper the safest thing to do about a thirteen-minute landing.
#
# It began as fifteen lines of inline `bash` in `ci.yml`, where it could not
# be tested -- which is its own small joke, since what it gates is the suite
# that exists to keep the tooling honest. `scripts/tests/test-ci-tooling-needed.py`
# is what tests it now.
#
# # The rule, and why it is an allow-list
#
# **Skip only when every changed path is provably unable to affect the
# tooling.** Anything else runs them. That direction is the whole design: a
# deny-list of "risky" paths would silently skip a top-level directory nobody
# has classified yet, and the failure would be a change to `scripts/` that
# nothing checked -- invisible, because the job is green and fast.
#
# Two prefixes are provably safe:
#
#   crates/  the workspace. No self-test reads it; they build sandboxes.
#   docs/    prose. `test-mutants-gate.py` is the one that would otherwise
#            read a real file under it, and it copies `mutants.sh` into its
#            sandbox precisely so the baseline it reads and writes is the
#            sandbox's -- it says so at length.
#
# `docs/` is here because it is what the measurements found: three landings
# in one session paid the eight minutes for an edit to `PRODUCT.md` or the
# generated config reference, on the critical path each time, and none of
# them could have changed a self-test's outcome.
set -euo pipefail

event=${1:-}

# Anything that is not a pull request -- a push to `main`, a manual dispatch
# -- runs them. There is no base to compare against, and `main` is the branch
# that must not be wrong.
if [ "$event" != "pull_request" ]; then
    echo yes
    exit 0
fi

files=$(cat)

# No input is "cannot prove anything", whether that is an API call that did
# not answer or a diff with nothing in it. Deliberately one case rather than
# two: a second channel saying which it was is a second thing to get wrong.
if [ -z "${files//[[:space:]]/}" ]; then
    echo yes
    exit 0
fi

if printf '%s\n' "$files" | grep -qvE '^(crates|docs)/'; then
    echo yes
else
    echo no
fi
