#!/usr/bin/env bash
# Run every repository invariant: the gate `issue-land.sh` runs before committing and CI runs. Each
# check in scripts/checks/ runs standalone.
# `check-no-personal-data.py` redacts by default (CI logs are public); run it with --reveal while
# fixing.
# A new invariant is a check-*.py in scripts/checks/ with a self-test in scripts/tests/.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# In parallel: one after another they took 63 s of a loaded landing's 95 s, and none writes what
# another reads. Each one's output is kept and printed in order, so a failure reads as it did.
JOBS="${POSTIO_CHECK_JOBS:-4}"
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

ls scripts/checks/check-*.py | xargs -P "$JOBS" -I{} sh -c '
    log="$2/$(basename "$1")"
    python3 "$1" > "$log.out" 2> "$log.err" || : > "$log.failed"' _ {} "$OUT"

FAILED=0
for check in scripts/checks/check-*.py; do
    log="$OUT/$(basename "$check")"
    cat "$log.out" 2>/dev/null
    cat "$log.err" >&2 2>/dev/null
    if [ -e "$log.failed" ] || [ ! -e "$log.out" ]; then
        echo "FAILED: $check" >&2
        FAILED=1
    fi
done

if [ "$FAILED" = 1 ]; then
    echo >&2
    echo "One or more invariants failed — see above. Each names its own fix." >&2
    exit 1
fi
echo "repository invariants: all clean"
