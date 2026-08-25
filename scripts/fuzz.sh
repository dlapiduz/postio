#!/usr/bin/env bash
# Run a fuzz target, from a shell that cannot run one unaided.
#
# Two things stand between `cargo fuzz run` and working here, and both produce
# errors that do not name their cause:
#
#   * libFuzzer needs nightly, and this workstation exports RUSTUP_TOOLCHAIN
#     from ~/.config/mise/config.toml. rustup reads the environment *before*
#     rust-toolchain.toml, so fuzz/rust-toolchain.toml is ignored and the build
#     fails with "the option `Z` is only accepted on the nightly compiler" --
#     which reads like a missing toolchain and is a winning environment
#     variable. See docs/engineering-notes.md.
#   * An unseeded corpus makes the fuzzer start from nothing, which for a MIME
#     parser means never generating a valid boundary. scripts/fuzz-seed.sh runs
#     first, every time; it is idempotent.
#
#   scripts/fuzz.sh parse_query                 # until you stop it
#   scripts/fuzz.sh parse_message -- -max_total_time=300
#   scripts/fuzz.sh --list
#
# Everything after `--` goes to libFuzzer. Everything before it goes to
# `cargo fuzz`.
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)

# rustup picks a toolchain file by the *working directory*, not by the
# manifest cargo was pointed at, so `--fuzz-dir` from the repo root still gets
# the root's 1.98.0 pin. Everything below runs from inside fuzz/.
cd "$REPO/fuzz"

# Checked before anything reaches cargo, because `cargo fuzz` with cargo-fuzz
# absent makes rustup download a whole nightly toolchain and *then* fail with
# "error: no such command: `fuzz`" -- which names neither the missing tool nor
# the fix, after several minutes that suggest the toolchain is the problem.
# A cargo subcommand is a `cargo-<name>` binary on PATH, so this is the same
# question cargo is about to ask, asked early enough to answer usefully. #277
# hit this trying to regenerate its own reproducer, which the issue's own
# instructions point at this script for.
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

# `env -u` rather than `unset`: this must not depend on how the caller's shell
# happens to be configured, and it leaves the caller's own environment alone.
exec env -u RUSTUP_TOOLCHAIN cargo fuzz run "$TARGET" "$@"
