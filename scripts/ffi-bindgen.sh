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
#   scripts/ffi-bindgen.sh [out-dir]        # default: target/ffi-bindings
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"
OUT="${1:-target/ffi-bindings}"

# The cdylib carries the metadata `uniffi-bindgen` reads in library mode, so it
# has to exist before the generator runs.
cargo build -p postio-ffi

LIB=""
for candidate in \
    "${CARGO_TARGET_DIR:-target}/debug/libpostio_ffi.dylib" \
    "${CARGO_TARGET_DIR:-target}/debug/libpostio_ffi.so"; do
    [ -f "$candidate" ] && LIB="$candidate" && break
done
if [ -z "$LIB" ]; then
    echo "no postio-ffi cdylib under ${CARGO_TARGET_DIR:-target}/debug." >&2
    echo "The [lib] crate-type in crates/postio-ffi/Cargo.toml must include" >&2
    echo "\`cdylib\` -- library-mode generation reads its metadata." >&2
    exit 1
fi

mkdir -p "$OUT"
cargo run -q -p postio-ffi --bin uniffi-bindgen -- \
    generate --library "$LIB" --language swift --out-dir "$OUT"

echo "swift bindings -> $OUT"
ls -1 "$OUT"
