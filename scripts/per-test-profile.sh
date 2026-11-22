#!/usr/bin/env bash
# One LLVM profile directory per test case: the nextest wrapper behind
# `scripts/test-coverage-sets.py` (specs/012-test-suite-value R2).
# nextest names each case in the environment, which a pid in LLVM_PROFILE_FILE cannot do; this
# points the profile at a directory named for the case,
# records the name beside it, and execs the test (on Linux the headless runner, `target-runner =
# "within-wrapper"`).
# The directory is `<binary id>/<first 16 hex of sha1(test name)>`; `case_dir()` in
# test-coverage-sets.py computes the same path, so keep them in step.
# Only the `per-test-coverage` nextest profile uses this.
set -euo pipefail

: "${NEXTEST_BINARY_ID:?not under nextest: run scripts/test-coverage-sets.py, which selects the per-test-coverage profile}"
: "${NEXTEST_TEST_NAME:?not under nextest: run scripts/test-coverage-sets.py, which selects the per-test-coverage profile}"

root=${NEXTEST_WORKSPACE_ROOT:-$PWD}
hash=$(printf %s "$NEXTEST_TEST_NAME" | { sha1sum 2>/dev/null || shasum; } | cut -c1-16)
dir="$root/target/test-audit/profiles/$NEXTEST_BINARY_ID/$hash"
mkdir -p "$dir"
printf '%s\n' "$NEXTEST_TEST_NAME" > "$dir/name"

# %m keeps the binary's module signature apart from a child's; %p keeps two processes of one binary
# apart.
export LLVM_PROFILE_FILE="$dir/%m-%p.profraw"
exec "$@"
