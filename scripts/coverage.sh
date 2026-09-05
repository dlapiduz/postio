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

# Measure against this tree, not against whatever is left in target/.
#
# cargo-llvm-cov derives coverage from instrumented artifacts and the .profraw
# files a run leaves behind. Reuse a target directory that was built from a
# different commit -- which is exactly what a CI cache with `restore-keys`
# hands you, since a prefix match restores an older key's directory -- and the
# measurement mixes stale region maps and stale profiles with current ones.
#
# It is not a small effect and it does not look like an error. The same branch
# measured postio-model at 94.25% on a run that missed the cache and 91.56% on
# the next run that hit it, with every test passing both times (#781). Floors
# compared against a number like that are measuring the cache, and the answer
# to a failure is to re-baseline, which ratchets the floor down to whatever
# the last restore happened to contain.
#
# Workspace artifacts only: the dependency graph is untouched, so this costs a
# rebuild of our own crates rather than of GTK and WebKit, and the cache goes
# on earning its keep for the part that is safe to reuse.
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

    # Kept apart from the parse below, and stderr kept rather than discarded.
    # When the measurement itself fails -- a crate that will not build under
    # instrumentation, a runner that ran out of memory -- `cargo llvm-cov`
    # writes nothing to stdout, and piping that straight into `json.load`
    # turned a tool failure into `JSONDecodeError: Expecting value: line 1
    # column 1`, a Python traceback naming neither the crate nor the reason.
    # That is what this looked like on CI, and it cost a run to find out.
    if ! report=$(env -u RUSTUP_TOOLCHAIN cargo llvm-cov -p "$crate" --json --summary-only); then
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
