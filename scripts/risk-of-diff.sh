#!/usr/bin/env bash
# How risky a diff is, from its paths alone: `high` or `normal`. Line one is the answer; for `high`,
# line two names the rule, so `issue-land.sh` can say why.
#
#     git diff --name-only origin/main...HEAD | scripts/risk-of-diff.sh
#     high
#     crates/postio-focus: the shared controller
#
# The bugs here live between layers that each pass, so a change's blast radius can exceed its diff:
# seeded mail (`seed.rs`, the `.eml` corpus),
# postio-focus (the shared controller), migrations, postio-sync, or 3+ crates.
# Build env (`rust-toolchain.toml`, `mise.toml`, `.cargo/config.toml`, `flatpak/*.json`) is not left
# to the author: a Rust bump to 1.100 passed every local
# gate and only the full suite saw Cargo move test binaries out of `deps/`, so line three,
# `full-suite`, makes `issue-land.sh` ask for it.
# Fails safe: an empty list is `high`, the same direction `ci-changes.sh` fails. Tested by
# `scripts/tests/test-risk-of-diff.py`.
set -euo pipefail

files=$(cat)
if [ -z "${files//[[:space:]]/}" ]; then
    printf 'high\nno diff to read: cannot show the change is small\n'
    exit 0
fi

if env_file=$(printf '%s\n' "$files" | grep -m1 -E '^(rust-toolchain\.toml|mise\.toml|\.cargo/config\.toml|flatpak/[^/]+\.json)$'); then
    printf 'high\n%s: the build environment\nfull-suite\n' "$env_file"
    exit 0
fi

high() { printf 'high\n%s\n' "$1"; exit 0; }

crates=""
while IFS= read -r file; do
    [ -n "$file" ] || continue
    case "$file" in
        crates/postio-storage/src/seed.rs) high "$file: the seeded demo mail" ;;
        *.eml)                             high "$file: fixture mail" ;;
        crates/postio-focus/*)             high "crates/postio-focus: the shared controller" ;;
        crates/postio-sync/*)              high "crates/postio-sync: the sync engine" ;;
        crates/postio-storage/src/schema.rs|crates/postio-storage/src/schema/*)
                                           high "$file: a storage migration" ;;
    esac
    crate=$(printf '%s' "$file" | sed -n 's|^crates/\([^/]*\)/.*|\1|p')
    [ -z "$crate" ] || crates=$(printf '%s\n%s' "$crates" "$crate")
done <<EOF_FILES
$files
EOF_FILES

count=$(printf '%s\n' "$crates" | sed '/^$/d' | sort -u | wc -l | tr -d ' ')
if [ "$count" -ge 3 ]; then
    high "$count crates touched (3 or more)"
fi
echo normal
