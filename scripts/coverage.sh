#!/usr/bin/env bash
# Measure line coverage for the pure/logic crates and gate it against
# scripts/coverage-floors.json.
#
# One crate at a time, never one workspace percentage: postio-gtk's tests
# need a compositor and will always read lower than postio-model's for
# reasons that say nothing about quality, and a single global number would
# hide a real regression in one crate behind an unrelated improvement in
# another. See #98 and scripts/coverage-floors.json's own comment.
#
# Runs entirely on this machine -- cargo-llvm-cov's own instrumented build,
# no upload, no third party ever sees a number. That is a requirement, not
# an incidental choice: this project's privacy posture is part of the
# product, and a coverage badge service is exactly the kind of remote
# request ADR 0011 §5 already ruled out for the docs site.
#
#   scripts/coverage.sh              # gate every crate in the floors file
#   scripts/coverage.sh postio-model # just one, still gated
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO"

FLOORS="$REPO/scripts/coverage-floors.json"

# Checked before anything reaches cargo, for the same reason scripts/fuzz.sh
# checks for cargo-fuzz first: an absent subcommand produces an error that
# names neither the missing tool nor the fix, several minutes into an
# instrumented rebuild of the workspace.
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "cargo-llvm-cov is not installed, so coverage cannot be measured." >&2
    echo >&2
    echo "    rustup component add llvm-tools-preview" >&2
    echo "    cargo install cargo-llvm-cov --locked" >&2
    echo >&2
    echo "Both are one-off; rustup honours rust-toolchain.toml for the rest." >&2
    exit 127
fi

if [ $# -gt 0 ]; then
    CRATES=("$@")
else
    mapfile -t CRATES < <(python3 -c "
import json
with open('$FLOORS') as f:
    floors = json.load(f)
for name in floors:
    if not name.startswith('_'):
        print(name)
")
fi

FAILED=0
for crate in "${CRATES[@]}"; do
    floor=$(python3 -c "
import json, sys
with open('$FLOORS') as f:
    floors = json.load(f)
if '$crate' not in floors:
    print('no floor recorded for \'$crate\' in scripts/coverage-floors.json', file=sys.stderr)
    sys.exit(1)
print(floors['$crate'])
")

    percent=$(env -u RUSTUP_TOOLCHAIN cargo llvm-cov -p "$crate" --json --summary-only 2>/dev/null \
        | python3 -c "import json, sys; print(json.load(sys.stdin)['data'][0]['totals']['lines']['percent'])")

    if python3 -c "import sys; sys.exit(0 if $percent >= $floor else 1)"; then
        printf 'ok:     %-16s %6.2f%% >= floor %5.2f%%\n' "$crate" "$percent" "$floor"
    else
        printf 'FAILED: %-16s %6.2f%% <  floor %5.2f%%\n' "$crate" "$percent" "$floor" >&2
        FAILED=1
    fi
done

if [ "$FAILED" = 1 ]; then
    echo >&2
    echo "Coverage dropped below a recorded floor. Add the missing tests, or if" >&2
    echo "the drop is deliberate (dead code removed, say), lower that crate's" >&2
    echo "floor in scripts/coverage-floors.json as its own reviewed change --" >&2
    echo "never as a side effect of an unrelated PR." >&2
    exit 1
fi

echo "coverage check passed (${#CRATES[@]} crate(s))."
