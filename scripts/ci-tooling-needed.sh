#!/usr/bin/env bash
# Whether CI needs to run the tooling self-tests for this change. Prints `yes` or `no`; reads
# changed paths on stdin, the GitHub event name is its argument.
# An allow-list: skip only when every changed path is provably unable to affect the tooling
# (`crates/`, which no self-test reads, and `docs/`).
# A deny-list would silently skip an unclassified top-level directory. Tested by
# `scripts/tests/test-ci-tooling-needed.py` (#996).
set -euo pipefail

event=${1:-}

# Anything but a pull request (a push to `main`, a dispatch) runs them: there is no base, and `main`
# must not be wrong.
if [ "$event" != "pull_request" ]; then
    echo yes
    exit 0
fi

files=$(cat)

# No input cannot prove anything, whether the API did not answer or the diff is empty; one case, not
# two.
if [ -z "${files//[[:space:]]/}" ]; then
    echo yes
    exit 0
fi

if printf '%s\n' "$files" | grep -qvE '^(crates|docs)/'; then
    echo yes
else
    echo no
fi
