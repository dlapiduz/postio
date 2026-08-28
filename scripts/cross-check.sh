#!/usr/bin/env bash
# Type-check the workspace for another platform, as far as this machine can.
#
# A Linux box cannot build or test the macOS half of Postio. It can do more
# than nothing, though, and the gap between "nothing" and "as far as it goes"
# is where #642 lived: a macOS-only dependency section swallowed fifteen
# entries and left `main` unbuildable for a day, with no gate that could have
# noticed.
#
# What this does: `cargo check --target <triple>` over every workspace member,
# and reports three outcomes rather than two.
#
#   ok       compiled for the target. Real coverage -- macOS-gated code in
#            this crate type-checks.
#   skipped  a C dependency's build script needs a cross-toolchain this
#            machine has not got (ring, zstd-sys, openssl-sys, the GTK sys
#            crates). Nothing about the Rust was learned; say so rather than
#            pretending.
#   FAILED   rustc rejected the code. This is the answer worth having.
#
# The split is deliberately *not* a hardcoded crate list. Which crates carry a
# C dependency changes, and a stale list would quietly stop checking something.
# Deciding per run from the actual failure keeps it honest.
#
# Usage:
#   scripts/cross-check.sh                          # aarch64-apple-darwin
#   scripts/cross-check.sh x86_64-apple-darwin      # another triple
#
# Setup, once:
#   rustup target add --toolchain "$(rustup show active-toolchain \
#     | cut -d' ' -f1)" aarch64-apple-darwin
#
# Not wired into check.sh on purpose: it compiles a second copy of the
# dependency graph, which is minutes on a cold target directory, and check.sh
# runs on every land across every session. This belongs in CI and in the
# reconcile pass. See docs/engineering-notes.md on cross-platform dependencies
# for the layers either side of it.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

TARGET="${1:-aarch64-apple-darwin}"

if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
    echo "cross-check: the $TARGET standard library is not installed." >&2
    echo >&2
    echo "  rustup target add --toolchain \"\$(rustup show active-toolchain | cut -d' ' -f1)\" $TARGET" >&2
    echo >&2
    echo "It is not in mise.toml: it is a large download that only this" >&2
    echo "script and CI need. Nothing was checked." >&2
    exit 2
fi

members=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))' \
    | sort)

ok=0
skipped=0
failed=0
failed_names=()
skipped_names=()

for crate in $members; do
    output=$(cargo check -q -p "$crate" --target "$TARGET" 2>&1)
    # Bash string matching rather than `grep -q` in a pipeline: `grep -q`
    # exits on the first match, the writer takes SIGPIPE, and `set -o
    # pipefail` reports that as the pipeline's status -- so the test reads
    # false exactly when the pattern *was* found. It cost two crates a
    # misclassification here before anybody noticed.
    if [[ $output != *$'\nerror'* && $output != error* ]]; then
        printf '  ok       %s\n' "$crate"
        ok=$((ok + 1))
    elif [[ $output == *"failed to run custom build command"* ]]; then
        # A C dependency, not our code. Name the crate that stopped us so the
        # reason is on the record rather than inferred.
        blocker=$(printf '%s\n' "$output" \
            | grep -oE 'custom build command for `[^`]+`' \
            | head -1 | sed 's/.*`\(.*\)`/\1/' || true)
        printf '  skipped  %-16s (needs a cross-toolchain for %s)\n' "$crate" "${blocker:-a C dependency}"
        skipped=$((skipped + 1))
        skipped_names+=("$crate")
    else
        printf '  FAILED   %s\n' "$crate"
        printf '%s\n' "$output" | grep -E '^error' -A 4 | head -20 || true
        failed=$((failed + 1))
        failed_names+=("$crate")
    fi
done

echo
echo "cross-check ($TARGET): $ok checked, $skipped skipped, $failed failed."

if [ "$skipped" -gt 0 ]; then
    echo
    echo "Not checked here, and only a $TARGET machine or a CI runner can:"
    printf '  %s\n' "${skipped_names[@]}"
fi

if [ "$failed" -gt 0 ]; then
    echo >&2
    echo "Code that does not compile for $TARGET:" >&2
    printf '  %s\n' "${failed_names[@]}" >&2
    exit 1
fi
