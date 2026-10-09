#!/usr/bin/env bash
# Build rustdoc for the crates this branch changed, under the nightly's flags.
#
# `issue-land.sh` already does this for every landing (#1463). This is the same
# build for a branch that does not land through it -- a pull request opened by
# hand from a host that cannot build every crate it touches, which is how the
# Mac sessions land GTK-touching work. Two of those PRs went red on CI's Docs
# job for a link rustdoc 1.99 rejects, after nothing local had built rustdoc
# with these flags.
#
#   scripts/doc-check.sh                 # the crates changed since origin/main
#   scripts/doc-check.sh postio-focus    # named crates
#
# Crates this host cannot build (scripts/unbuildable-crates.sh) are skipped,
# and named as skipped: CI documents them.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ $# -gt 0 ]; then
    CRATES="$*"
else
    CRATES=$( { git diff --name-only origin/main...HEAD; git status --porcelain | sed 's/^...//'; } \
        | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u | tr '\n' ' ')
fi
UNBUILDABLE=" $(scripts/unbuildable-crates.sh | tr '\n' ' ') "

ARGS=()
NAMES=""
SKIPPED=""
for crate in $CRATES; do
    case "$UNBUILDABLE" in
        *" $crate "*) SKIPPED="${SKIPPED:+$SKIPPED }$crate" ;;
        *) ARGS+=(-p "$crate"); NAMES="${NAMES:+$NAMES }$crate" ;;
    esac
done

[ -n "$SKIPPED" ] && echo "doc-check: this host cannot build $SKIPPED; CI documents them." >&2
if [ ${#ARGS[@]} -eq 0 ]; then
    echo "doc-check: no buildable crate changed; nothing to document."
    exit 0
fi
RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" \
    cargo doc --no-deps --document-private-items "${ARGS[@]}"
echo "doc-check: rustdoc clean for $NAMES"
