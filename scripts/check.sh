#!/usr/bin/env bash
# Run every repository invariant. This is THE gate command: issue-land.sh runs
# it before committing, CI runs the same checks, and running it by hand is how
# you ask "is this tree clean?" without remembering seven script names.
#
# Each check lives in scripts/checks/ and runs standalone too — useful when
# fixing one violation. check-no-personal-data.py redacts what it finds by
# default (CI logs are public); run it directly with --reveal while fixing:
#
#   python3 scripts/checks/check-no-personal-data.py --reveal
#
# Adding an invariant = dropping a check-*.py into scripts/checks/ (with a
# self-test in scripts/tests/). Nothing else to wire up.
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
