#!/usr/bin/env bash
# Regenerate the Swift bindings for `postio-ffi`.
#
# The bindings are a build product and are not tracked: the generator and the
# `uniffi` runtime must be the same version or the Swift compiles against
# nothing, and the only way to guarantee that is to build both from this
# workspace every time. A checked-in copy would be a third thing to keep in
# step, and the failure it causes -- a checksum mismatch at app startup -- is
# far away from the change that caused it.
#
# Usage:
#   scripts/ffi-bindgen.sh [--release] [out-dir]   # default: target/ffi-bindings
#
# `--release` reads the release library, for `macos-build.sh --release`. It
# used to read a debug one whatever the caller built, so a release build of
# the Mac app compiled the workspace a second time, in debug, only to read
# the same metadata: twenty of the macOS release job's sixty minutes
# (2026-11-16).
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"
PROFILE=debug
PROFILE_ARGS=()
OUT=target/ffi-bindings
for arg in "$@"; do
    case "$arg" in
        --release) PROFILE=release; PROFILE_ARGS=(--release) ;;
        -*) echo "unknown argument: $arg" >&2; exit 2 ;;
        *) OUT="$arg" ;;
    esac
done

# The cdylib carries the metadata `uniffi-bindgen` reads in library mode, so it
# has to exist before the generator runs. Built with whatever features the
# caller built it with (`POSTIO_FFI_FEATURES`, as macos-build.sh takes it):
# building it here without them would put a library without them back where
# the bundle copies it from.
FEATURE_ARGS=()
if [ -n "${POSTIO_FFI_FEATURES:-}" ]; then
    FEATURE_ARGS=(--features "$POSTIO_FFI_FEATURES")
fi
cargo build -p postio-ffi ${PROFILE_ARGS[@]+"${PROFILE_ARGS[@]}"} ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"}

LIB=""
for candidate in \
    "${CARGO_TARGET_DIR:-target}/${PROFILE}/libpostio_ffi.dylib" \
    "${CARGO_TARGET_DIR:-target}/${PROFILE}/libpostio_ffi.so"; do
    [ -f "$candidate" ] && LIB="$candidate" && break
done
if [ -z "$LIB" ]; then
    echo "no postio-ffi cdylib under ${CARGO_TARGET_DIR:-target}/${PROFILE}." >&2
    echo "The [lib] crate-type in crates/postio-ffi/Cargo.toml must include" >&2
    echo "\`cdylib\` -- library-mode generation reads its metadata." >&2
    exit 1
fi

mkdir -p "$OUT"
cargo run -q -p postio-ffi ${PROFILE_ARGS[@]+"${PROFILE_ARGS[@]}"} ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"} --bin uniffi-bindgen -- \
    generate --library "$LIB" --language swift --out-dir "$OUT"

echo "swift bindings -> $OUT"
ls -1 "$OUT"
