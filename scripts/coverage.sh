#!/usr/bin/env bash
# Measure line coverage for the pure/logic crates and gate it against scripts/coverage-floors.json.
# One crate at a time, never one workspace percentage: GTK crates read lower for reasons unrelated
# to quality (#98).
# Entirely local: ADR 0011 §5 rules out remote coverage services.
#
#   scripts/coverage.sh              # gate every crate in the floors file
#   scripts/coverage.sh postio-model # just one, still gated
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO"

FLOORS="$REPO/scripts/coverage-floors.json"

# Checked before cargo: an absent subcommand errors without naming the tool, minutes into an
# instrumented rebuild.
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

# Measure against this tree: a target dir from another commit (a CI `restore-keys` hit) mixes stale
# region maps and profiles, giving 94.25% vs 91.56% on the same branch (#781). Only workspace
# artifacts are cleared, so the dependency cache still pays.
echo "clearing stale coverage artifacts for this workspace..."
env -u RUSTUP_TOOLCHAIN cargo llvm-cov clean --workspace

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

    # stderr kept apart from the parse: a failed measurement writes nothing to stdout, and piping
    # that to `json.load` gave an opaque JSONDecodeError.
    # Through nextest, not `cargo test`: only nextest's `default-filter` keeps the measurement tier
    # off this job (#1456).
    if ! report=$(env -u RUSTUP_TOOLCHAIN cargo llvm-cov nextest -p "$crate" --json --summary-only); then
        echo >&2
        echo "could not measure coverage for '$crate': cargo llvm-cov failed." >&2
        echo "Its error is above; this is a broken measurement, not a floor." >&2
        exit 1
    fi
    if [ -z "$report" ]; then
        echo >&2
        echo "cargo llvm-cov produced no report for '$crate'." >&2
        exit 1
    fi
    percent=$(printf '%s' "$report" \
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
