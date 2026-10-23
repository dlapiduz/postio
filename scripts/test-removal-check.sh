#!/usr/bin/env bash
# If these test cases went, would any mutant they catch go uncaught?
#
# Spec 012's third question (specs/012-test-suite-value, US3), and the only
# licence to remove a test under it (FR-007). Coverage can say two cases run
# the same code; it cannot say they assert the same things. Mutation can:
# a case is redundant exactly when every mutant it catches, something else
# catches too.
#
# Two passes over the named source files (research R4):
#
#   A. cargo-mutants with the suite *without* the cases -- a nextest `-E`
#      filter through `--cargo-test-arg`, which is how cargo-mutants passes
#      arguments to the test tool;
#   B. only the mutants A missed that docs/mutants-baseline.txt does not
#      already list, re-run *with* the cases.
#
# A mutant B catches is one only these cases catch: a new survivor if they
# go. None, and the verdict is "safe". B costs what A missed, not a second
# full run. The baseline diff is `comm` over sorted lines, as
# scripts/mutants.sh does it.
#
# Mutation is slow and heavy -- a rebuild and a test run per mutant -- and
# scripts/mutants.sh records what it did to a shared workstation. So this
# refuses to run where CI is unset, unless POSTIO_ALLOW_MUTANTS_HERE=1 says
# this machine is yours. On CI, `.github/workflows/test-audit.yml` runs it.
#
#   scripts/test-removal-check.sh <source file>... -- '<binary id> <test name>'...
#
# Writes target/test-audit/verdict.json. Exit status: 0 safe; 1 not safe;
# 2 bad arguments, a missing tool, or refused to run here.
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
BASELINE=${MUTANTS_BASELINE:-$REPO/docs/mutants-baseline.txt}
OUT=${TEST_AUDIT_OUT:-$REPO/target/test-audit}

usage() {
    echo "usage: scripts/test-removal-check.sh <source file>... -- '<binary id> <test name>'..." >&2
    echo "  e.g. scripts/test-removal-check.sh crates/postio-config/src/keys.rs -- \\" >&2
    echo "         'postio-config keys::mod_token_tests::ctrl_stays_literal_on_both'" >&2
    exit 2
}

FILES=()
CASES=()
seen_separator=
for arg in "$@"; do
    if [ "$arg" = "--" ] && [ -z "$seen_separator" ]; then
        seen_separator=1
    elif [ -z "$seen_separator" ]; then
        FILES+=("$arg")
    else
        CASES+=("$arg")
    fi
done
[ -n "$seen_separator" ] && [ ${#FILES[@]} -gt 0 ] && [ ${#CASES[@]} -gt 0 ] || usage

if [ -z "${CI:-}" ] && [ -z "${POSTIO_ALLOW_MUTANTS_HERE:-}" ]; then
    echo "Refusing to mutate code on a machine other sessions may share." >&2
    echo >&2
    echo "scripts/mutants.sh records why: a full run \"briefly drove this shared" >&2
    echo "workstation's load average past 14\". Run this on CI instead --" >&2
    echo >&2
    echo "    gh workflow run test-audit.yml -f mode=removal -f files='${FILES[*]}' -f cases='...'" >&2
    echo >&2
    echo "-- or, on a machine that is yours alone, set POSTIO_ALLOW_MUTANTS_HERE=1." >&2
    exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is not on PATH." >&2
    exit 2
fi

# `not ((binary_id(=a) & test(=x)) | (binary_id(=b) & test(=y)))`
filter=
for case in "${CASES[@]}"; do
    binary=${case%% *}
    name=${case#* }
    term="(binary_id(=$binary) & test(=$name))"
    filter=${filter:+$filter | }$term
done
filter="not ($filter)"

FILE_ARGS=()
for file in "${FILES[@]}"; do
    FILE_ARGS+=(-f "$file")
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"

# --no-shuffle and --jobs 1 for the reasons scripts/mutants.sh gives.
env -u RUSTUP_TOOLCHAIN cargo mutants --test-tool=nextest --no-shuffle --jobs 1 "${FILE_ARGS[@]}" \
    --output "$WORK/a" --cargo-test-arg=-E "--cargo-test-arg=$filter" || true
if [ ! -f "$WORK/a/mutants.out/missed.txt" ]; then
    echo "Pass A produced no missed.txt; see $WORK/a/mutants.out for why." >&2
    trap - EXIT
    exit 2
fi

touch "$WORK/baseline"
[ -f "$BASELINE" ] && sort "$BASELINE" > "$WORK/baseline"
sort "$WORK/a/mutants.out/missed.txt" > "$WORK/a-missed"
comm -13 "$WORK/baseline" "$WORK/a-missed" > "$WORK/new-misses"

: > "$WORK/survivors"
if [ -s "$WORK/new-misses" ]; then
    # One --re per mutant: its file and its description, wherever cargo-mutants puts the line and column.
    RE_ARGS=()
    while IFS= read -r mutant; do
        RE_ARGS+=(--re "$(python3 -c 'import re,sys; f, _, rest = sys.argv[1].partition(":"); d = rest.split(": ", 1)[-1]; print(re.escape(f) + ".*" + re.escape(d))' "$mutant")")
    done < "$WORK/new-misses"
    env -u RUSTUP_TOOLCHAIN cargo mutants --test-tool=nextest --no-shuffle --jobs 1 "${FILE_ARGS[@]}" \
        --output "$WORK/b" "${RE_ARGS[@]}" || true
    if [ -f "$WORK/b/mutants.out/caught.txt" ]; then
        sort "$WORK/b/mutants.out/caught.txt" > "$WORK/b-caught"
        comm -12 "$WORK/new-misses" "$WORK/b-caught" > "$WORK/survivors"
    else
        echo "Pass B produced no caught.txt; see $WORK/b/mutants.out for why." >&2
        trap - EXIT
        exit 2
    fi
fi

commit=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)
python3 - "$OUT/verdict.json" "$WORK/survivors" "$commit" "${#FILES[@]}" "${FILES[@]}" "${CASES[@]}" <<'PY'
import json, sys
out, survivors, commit, nfiles = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
files, cases = sys.argv[5 : 5 + nfiles], sys.argv[5 + nfiles :]
lost = [l for l in open(survivors).read().splitlines() if l]
json.dump({"cases": cases, "files": files, "new_survivors": lost,
           "result": "not safe" if lost else "safe", "commit": commit}, open(out, "w"), indent=1)
PY

count=$(wc -l < "$WORK/survivors" | tr -d ' ')
if [ "$count" -eq 0 ]; then
    echo "safe: removing ${#CASES[@]} case(s) leaves no new surviving mutant in ${#FILES[@]} file(s) (commit $commit)"
    exit 0
fi
echo "NOT SAFE: $count mutant(s) survive without these cases and were caught with them:"
sed 's/^/  /' "$WORK/survivors"
echo "Keep the case that catches it, or move its assertion to the cheapest layer that can see it (ADR 0049)."
exit 1
