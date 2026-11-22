#!/usr/bin/env bash
# Regenerate the Swift bindings for `postio-ffi`. They are a build product, not tracked: the
# generator and the `uniffi` runtime must be the same version, so both are built from this
# workspace.
#
# Usage:
#   scripts/ffi-bindgen.sh [--release] [out-dir]   # default: target/ffi-bindings
#
# `--release` reads the release library, so a release Mac build does not compile the workspace again
# in debug.
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

# The cdylib carries the metadata `uniffi-bindgen` reads; build it with the caller's features
# (`POSTIO_FFI_FEATURES`), or the bundle copies one without them.
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
