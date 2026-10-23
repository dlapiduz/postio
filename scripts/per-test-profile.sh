#!/usr/bin/env bash
# One LLVM profile directory per test case: the nextest wrapper behind
# `scripts/test-coverage-sets.py` (specs/012-test-suite-value, research R2).
#
# nextest runs every case in its own process and names it in the
# environment. `%p` in LLVM_PROFILE_FILE would give a profile per process,
# but nothing maps a pid back to a test; these two variables do. So this
# points the profile at a directory named for the case, records the name
# beside it, and execs the test -- or, on Linux, the headless runner that
# execs the test (`target-runner = "within-wrapper"` in .config/nextest.toml).
#
# The directory is `<binary id>/<first 16 hex of sha1(test name)>`: test
# names hold `::` and can be long. `case_dir()` in test-coverage-sets.py
# computes the same path; keep the two in step.
#
# Only the `per-test-coverage` nextest profile uses this. Nothing on the
# merge path does.
set -euo pipefail

: "${NEXTEST_BINARY_ID:?not under nextest: run scripts/test-coverage-sets.py, which selects the per-test-coverage profile}"
: "${NEXTEST_TEST_NAME:?not under nextest: run scripts/test-coverage-sets.py, which selects the per-test-coverage profile}"

root=${NEXTEST_WORKSPACE_ROOT:-$PWD}
hash=$(printf %s "$NEXTEST_TEST_NAME" | { sha1sum 2>/dev/null || shasum; } | cut -c1-16)
dir="$root/target/test-audit/profiles/$NEXTEST_BINARY_ID/$hash"
mkdir -p "$dir"
printf '%s\n' "$NEXTEST_TEST_NAME" > "$dir/name"

# %m keeps the binary's own module signature apart from a child process's;
# %p keeps two processes of the same binary apart.
export LLVM_PROFILE_FILE="$dir/%m-%p.profraw"
exec "$@"
