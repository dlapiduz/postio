#!/usr/bin/env bash
# Mutation-test the pure/logic crates: postio-model, postio-search, postio-config and postio-sync.
# Not the GTK crates: mutating widget code gives timeouts and noise. Slow by construction (a full
# rebuild and test per mutant, ~1900 mutants, hours), so it never gates a PR and belongs on a runner
# of its own, never a shared workstation. It reports survivors past docs/mutants-baseline.txt (#99).
#
#   scripts/mutants.sh                          # every crate above
#   scripts/mutants.sh postio-config            # just one
#   MUTANTS_UPDATE_BASELINE=1 scripts/mutants.sh  # reseed after triage
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO"

BASELINE="$REPO/docs/mutants-baseline.txt"

# Checked first: an absent subcommand should not be discovered minutes into an instrumented rebuild.
if ! command -v cargo-mutants >/dev/null 2>&1; then
    echo "cargo-mutants is not installed, so mutation testing cannot run." >&2
    echo >&2
    echo "    cargo install cargo-mutants --locked" >&2
    exit 127
fi

if [ $# -gt 0 ]; then
    CRATES=("$@")
else
    CRATES=(postio-model postio-search postio-config postio-sync)
fi

PACKAGE_ARGS=()
for crate in "${CRATES[@]}"; do
    PACKAGE_ARGS+=(-p "$crate")
done

OUTPUT_DIR=$(mktemp -d)
trap 'rm -rf "$OUTPUT_DIR"' EXIT

# --no-shuffle: a stable order makes two runs comparable against the baseline.
# --jobs 1: cargo-mutants gives each parallel job its own tree copy and fresh target/, and `--jobs
# 2` still lost the hosted runner at ~1h50m (#510);
# one job trades wall time for peak memory and disk. If it still dies, split `mutants.yml` per
# crate.
env -u RUSTUP_TOOLCHAIN cargo mutants "${PACKAGE_ARGS[@]}" --no-shuffle --jobs 1 \
    --output "$OUTPUT_DIR" || true

SURVIVED="$OUTPUT_DIR/mutants.out/missed.txt"
if [ ! -f "$SURVIVED" ]; then
    echo "cargo-mutants produced no missed.txt; something upstream of the" >&2
    echo "report failed. See $OUTPUT_DIR/mutants.out/ for the raw logs." >&2
    exit 1
fi

if [ -n "${MUTANTS_UPDATE_BASELINE:-}" ]; then
    # Rewrite only the crates this run covered and keep the rest: the baseline is one file and
    # `mutants.yml` runs a job per crate (#1458), so a whole-file rewrite lets the last shard win.
    # A line is a crate's when it begins `crates/<name>/`; an unclassifiable line is kept, since
    # dropping it turns the baseline into a record of the last run.
    KEPT=$(mktemp)
    trap 'rm -rf "$OUTPUT_DIR" "$KEPT"' EXIT
    if [ -f "$BASELINE" ]; then
        cp "$BASELINE" "$KEPT"
        for crate in "${CRATES[@]}"; do
            grep -v "^crates/$crate/" "$KEPT" > "$KEPT.next" || true
            mv "$KEPT.next" "$KEPT"
        done
    else
        : > "$KEPT"
    fi
    sort -u "$KEPT" "$SURVIVED" > "$BASELINE"
    echo "wrote $(wc -l < "$BASELINE") surviving mutant(s) to $BASELINE" \
         "($(wc -l < "$SURVIVED") from this run over ${CRATES[*]};" \
         "$(wc -l < "$KEPT") kept from other crates)"
    exit 0
fi

if [ ! -f "$BASELINE" ]; then
    echo "no baseline recorded yet at $BASELINE." >&2
    echo >&2
    echo "This run found $(wc -l < "$SURVIVED") surviving mutant(s):" >&2
    echo >&2
    sort "$SURVIVED" >&2
    echo >&2
    echo "Read them, file an issue for any that are a genuine missing test," >&2
    echo "then seed the baseline with what remains -- on a CI runner, not a" >&2
    echo "workstation other sessions share (see this script's own header):" >&2
    echo >&2
    echo "    MUTANTS_UPDATE_BASELINE=1 scripts/mutants.sh" >&2
    exit 1
fi

NEW_SURVIVORS=$(comm -13 <(sort "$BASELINE") <(sort "$SURVIVED"))

if [ -n "$NEW_SURVIVORS" ]; then
    echo "New surviving mutants, past the committed baseline:" >&2
    echo "$NEW_SURVIVORS" >&2
    echo >&2
    echo "Each one means a change went in with no test noticing it. Add the" >&2
    echo "missing test, or if the survivor is genuinely uninteresting (an" >&2
    echo "unreachable arm, a Debug impl), add it to docs/mutants-baseline.txt" >&2
    echo "as its own reviewed change." >&2
    exit 1
fi

echo "mutation check passed: no new survivors past the recorded baseline."
