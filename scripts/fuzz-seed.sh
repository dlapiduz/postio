#!/usr/bin/env bash
# Fill fuzz/corpus/<target>/ with starting inputs.
#
# libFuzzer works by mutating what it already has, so the corpus decides how
# long it takes to reach anything interesting. Starting from real mail rather
# than from random bytes is the difference between finding a charset bug in
# minutes and never generating a valid MIME boundary at all.
#
# Nothing here is generated *into* git: fuzz/corpus/ is ignored, and this
# script is what recreates it. The `.eml` fixtures stay in one place --
# crates/postio-model/tests/corpus, where the /add-fixture skill maintains
# them -- rather than being copied into the tree twice and drifting.
#
# Idempotent, and safe to run over a corpus libFuzzer has already grown: it
# adds seeds and never removes anything it finds.
#
#   scripts/fuzz-seed.sh            # all three targets
#   scripts/fuzz-seed.sh parse_query
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
CORPUS="$REPO/fuzz/corpus"
EML="$REPO/crates/postio-model/tests/corpus"
SEEDS="$REPO/fuzz/seeds"

# Copy every file in $1 into the corpus of target $2, named by content hash so
# re-running never makes duplicates and two sources cannot collide on a name.
seed_from() {
    local from="$1" target="$2" count=0
    mkdir -p "$CORPUS/$target"
    [ -d "$from" ] || return 0
    for file in "$from"/*; do
        [ -f "$file" ] || continue
        local hash
        hash=$(sha256sum "$file" | cut -c1-16)
        if [ ! -f "$CORPUS/$target/$hash" ]; then
            cp "$file" "$CORPUS/$target/$hash"
            count=$((count + 1))
        fi
    done
    echo "  $target: +$count from ${from#"$REPO"/}"
}

seed_parse_message() { seed_from "$EML" parse_message; }
seed_sanitize_html() {
    # Both the hand-written hostile markup and the real messages: a `.eml` is
    # not HTML, but the fuzzer only needs bytes that contain the shapes it
    # should be mutating, and the corpus carries markup no seed file has.
    seed_from "$SEEDS/html" sanitize_html
    seed_from "$EML" sanitize_html
}
seed_parse_query() { seed_from "$SEEDS/query" parse_query; }

TARGETS=("${@:-parse_message sanitize_html parse_query}")
# shellcheck disable=SC2068 # deliberate: the default above is a word list.
for target in ${TARGETS[@]}; do
    case "$target" in
        parse_message) seed_parse_message ;;
        sanitize_html) seed_sanitize_html ;;
        parse_query)   seed_parse_query ;;
        *) echo "unknown target: $target" >&2; exit 2 ;;
    esac
done

echo "seeded. run one with:  cd fuzz && cargo fuzz run <target>"
