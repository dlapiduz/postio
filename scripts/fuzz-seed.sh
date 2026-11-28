#!/usr/bin/env bash
# Fill fuzz/corpus/<target>/ with starting inputs: real mail, not random bytes, so libFuzzer reaches
# valid MIME.
# fuzz/corpus/ is ignored; this recreates it from crates/postio-model/tests/corpus. Idempotent: it
# adds seeds and never removes.
#
#   scripts/fuzz-seed.sh            # every target
#   scripts/fuzz-seed.sh parse_query
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
CORPUS="$REPO/fuzz/corpus"
EML="$REPO/crates/postio-model/tests/corpus"
SEEDS="$REPO/fuzz/seeds"

# Copy every file in $1 into target $2's corpus, named by content hash so re-runs make no
# duplicates.
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
    # Both the hostile markup and the real messages: the fuzzer only needs bytes containing the
    # shapes to mutate.
    seed_from "$SEEDS/html" sanitize_html
    seed_from "$EML" sanitize_html
}
seed_parse_query() { seed_from "$SEEDS/query" parse_query; }
seed_verify_signature() { seed_from "$EML" verify_signature; }
# The keys themselves, for the readers to find keyblocks in; a mutated keybox is the hostile case.
seed_read_keyring() { seed_from "$EML/keys" read_keyring; }

TARGETS=("${@:-parse_message sanitize_html parse_query verify_signature read_keyring}")
# shellcheck disable=SC2068 # deliberate: the default above is a word list.
for target in ${TARGETS[@]}; do
    case "$target" in
        parse_message) seed_parse_message ;;
        sanitize_html) seed_sanitize_html ;;
        parse_query)   seed_parse_query ;;
        verify_signature) seed_verify_signature ;;
        read_keyring)  seed_read_keyring ;;
        *) echo "unknown target: $target" >&2; exit 2 ;;
    esac
done

echo "seeded. run one with:  cd fuzz && cargo fuzz run <target>"
