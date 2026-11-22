#!/usr/bin/env bash
# Run a fuzz target from a shell that cannot run one unaided. The workstation exports
# RUSTUP_TOOLCHAIN, which rustup reads before rust-toolchain.toml, so the nightly pin is ignored;
# the script unsets it. scripts/fuzz-seed.sh runs first (idempotent).
#
#   scripts/fuzz.sh parse_query                 # until you stop it
#   scripts/fuzz.sh parse_message -- -max_total_time=300
#   scripts/fuzz.sh --list
#
# Everything after `--` goes to libFuzzer; everything before it to `cargo fuzz`.
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)

# rustup picks a toolchain file by the working directory, not the manifest, so everything runs from
# inside fuzz/.
cd "$REPO/fuzz"

# Checked before cargo: with cargo-fuzz absent, rustup downloads a whole nightly and then fails with
# an error naming neither the tool nor the fix (#277).
if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz is not installed, so no fuzz target can run here." >&2
    echo >&2
    echo "    cargo install cargo-fuzz" >&2
    echo >&2
    echo "It is a one-off: the nightly toolchain fuzz/rust-toolchain.toml pins" >&2
    echo "is fetched on the first run, and this script already keeps this" >&2
    echo "workstation's RUSTUP_TOOLCHAIN from overriding it." >&2
    exit 127
fi

if [ "${1:-}" = "--list" ]; then
    env -u RUSTUP_TOOLCHAIN cargo fuzz list
    exit 0
fi

if [ $# -eq 0 ]; then
    echo "usage: scripts/fuzz.sh <target> [cargo-fuzz args] [-- libfuzzer args]" >&2
    echo "targets:" >&2
    env -u RUSTUP_TOOLCHAIN cargo fuzz list | sed 's/^/  /' >&2
    exit 2
fi

TARGET="$1"; shift
"$REPO/scripts/fuzz-seed.sh" "$TARGET"

# `env -u` rather than `unset`: it leaves the caller's own environment alone.
exec env -u RUSTUP_TOOLCHAIN cargo fuzz run "$TARGET" "$@"
