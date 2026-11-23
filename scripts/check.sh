#!/usr/bin/env bash
# Run every repository invariant: the gate `issue-land.sh` runs before committing and CI runs. Each
# check in scripts/checks/ runs standalone.
# `check-no-personal-data.py` redacts by default (CI logs are public); run it with --reveal while
# fixing.
# A new invariant is a check-*.py in scripts/checks/ with a self-test in scripts/tests/.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

FAILED=0
for check in scripts/checks/check-*.py; do
    if ! python3 "$check"; then
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
