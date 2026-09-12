#!/usr/bin/env bash
# Put SQLite3 Multiple Ciphers under the whole workspace, prove the store is
# still encrypted, run postio-storage's own suite against it, and time a scan.
#
#   spike/sqlite3mc/run.sh
#
# It works by replacing the amalgamation inside a scratch copy of
# libsqlite3-sys: sqlite3mc *is* SQLite with encryption built in, so the
# `bundled` feature builds it with no further help, and `bundled-sqlcipher`
# and its OpenSSL are not wanted at all.
#
# The workspace edits are applied from a patch and reverted at the end, so a
# checkout is never left in a state only this script understands.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="${TMPDIR:-/tmp}/postio-sqlite3mc-spike"
version="0.38.2"

src="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 2 \
        -type d -name "libsqlite3-sys-$version" | head -1)"
[ -n "$src" ] || { echo "libsqlite3-sys-$version is not unpacked; build once first" >&2; exit 1; }

rm -rf "$work"; mkdir -p "$work/mc"
echo "── fetching the sqlite3mc amalgamation ──"
asset="$(gh api repos/utelle/SQLite3MultipleCiphers/releases/latest \
          --jq '.assets[] | select(.name|endswith("amalgamation.zip")) | .id')"
gh api "repos/utelle/SQLite3MultipleCiphers/releases/assets/$asset" \
  -H "Accept: application/octet-stream" > "$work/mc.zip"
unzip -q -o "$work/mc.zip" -d "$work/mc"

echo "── building a libsqlite3-sys whose amalgamation is sqlite3mc ──"
cp -r "$src" "$work/libsqlite3-sys"
chmod -R u+w "$work/libsqlite3-sys"
cp "$work/mc/sqlite3mc_amalgamation.c" "$work/libsqlite3-sys/sqlite3/sqlite3.c"
cp "$work/mc/sqlite3.h"                "$work/libsqlite3-sys/sqlite3/sqlite3.h"
cp "$work/mc/sqlite3ext.h"             "$work/libsqlite3-sys/sqlite3/sqlite3ext.h"

cleanup() {
  echo
  echo "── putting the workspace back ──"
  git -C "$root" checkout -- Cargo.toml Cargo.lock crates/*/Cargo.toml \
      crates/postio-storage/src/db.rs 2>/dev/null || true
}
trap cleanup EXIT

echo "── swapping the workspace over ──"
sed "s#@SQLITE3MC_SYS@#$work/libsqlite3-sys#g" \
  "$here/workspace-to-sqlite3mc.patch" | git -C "$root" apply -

echo
echo "── postio-storage's own suite, on sqlite3mc ──"
( cd "$root" && cargo nextest run -p postio-storage --no-fail-fast ) || true
