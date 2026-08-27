#!/usr/bin/env bash
# Mutation-test the pure/logic crates: postio-model, postio-search,
# postio-config, and postio-sync (the reconciliation logic lives there).
# Not postio-gtk -- mutating widget code produces mostly timeouts and noise
# on tests that need a compositor.
#
# CLAUDE.md already asks every session to do this by hand: "verify your
# tests can fail" by injecting the regression a test exists to catch and
# confirming it goes red. Mutation testing is that instruction, mechanised
# and run against everything at once instead of one test at a time.
#
# It is slow by construction -- a full rebuild and test run per mutant --
# so this never gates a PR. It runs nightly (scripts/mutants.sh is what
# .github/workflows/mutants.yml calls) and its job is to report surviving
# mutants past the committed baseline in docs/mutants-baseline.txt, not to
# fail a build on every one it finds. See #99.
#
# On ~1900 mutants across these four crates, a full run is measured in
# hours, not minutes, and briefly drove this shared workstation's load
# average past 14 -- so run it on a runner of its own (the nightly job),
# never on a machine other sessions share. The same POSTIO_UPDATE_DOCS=1
# idiom keybindings_doc.rs and config_doc.rs use reseeds the baseline once
# a run finishes:
#
#   scripts/mutants.sh                          # every crate above
#   scripts/mutants.sh postio-config            # just one
#   MUTANTS_UPDATE_BASELINE=1 scripts/mutants.sh  # reseed after triage
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO"

BASELINE="$REPO/docs/mutants-baseline.txt"

# Checked first, same reason as scripts/coverage.sh and scripts/fuzz.sh: an
# absent subcommand should not be discovered several minutes into a full
# instrumented rebuild.
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

# --no-shuffle: a stable order makes two runs of the same tree comparable,
# which matters once this is diffed against a baseline rather than just read.
env -u RUSTUP_TOOLCHAIN cargo mutants "${PACKAGE_ARGS[@]}" --no-shuffle \
    --output "$OUTPUT_DIR" || true

SURVIVED="$OUTPUT_DIR/mutants.out/missed.txt"
if [ ! -f "$SURVIVED" ]; then
    echo "cargo-mutants produced no missed.txt; something upstream of the" >&2
    echo "report failed. See $OUTPUT_DIR/mutants.out/ for the raw logs." >&2
    exit 1
fi

if [ -n "${MUTANTS_UPDATE_BASELINE:-}" ]; then
    sort "$SURVIVED" > "$BASELINE"
    echo "wrote $(wc -l < "$BASELINE") surviving mutant(s) to $BASELINE"
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

# A line survives the diff against the baseline the same way any other
# generated-reference check in this repository does: sorted, compared
# exactly, new lines are what is new.
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
