#!/usr/bin/env bash
# Put SQLite3 Multiple Ciphers where this branch's [patch.crates-io] expects
# it: vendor/libsqlite3-sys, which is gitignored.
#
#   spike/sqlite3mc/setup.sh
#
# **This branch does not build until you run it.** That is deliberate rather
# than unfinished: the alternative is a 13 MB amalgamation committed to git,
# and the upstream publishes signed SHA256SUMS, so fetching and verifying is
# the better provenance as well as the smaller diff.
#
# sqlite3mc *is* SQLite with encryption built in, so nothing here needs the
# `bundled-sqlcipher` feature or OpenSSL: the plain `bundled` path compiles
# the amalgamation this drops in.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
vendor="$root/vendor/libsqlite3-sys"
version="0.38.2"

src="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 2 \
        -type d -name "libsqlite3-sys-$version" | head -1)"
[ -n "$src" ] || {
  echo "libsqlite3-sys-$version is not unpacked in the registry." >&2
  echo "Build any crate that uses rusqlite once, then run this again." >&2
  exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "── fetching the sqlite3mc amalgamation ──"
tag="$(gh api repos/utelle/SQLite3MultipleCiphers/releases/latest --jq .tag_name)"
asset="$(gh api repos/utelle/SQLite3MultipleCiphers/releases/latest \
          --jq '.assets[] | select(.name|endswith("amalgamation.zip")) | .id')"
name="$(gh api repos/utelle/SQLite3MultipleCiphers/releases/latest \
          --jq '.assets[] | select(.name|endswith("amalgamation.zip")) | .name')"
gh api "repos/utelle/SQLite3MultipleCiphers/releases/assets/$asset" \
  -H "Accept: application/octet-stream" > "$work/mc.zip"

echo "── checking it against the published sums ──"
sums="$(gh api repos/utelle/SQLite3MultipleCiphers/releases/latest \
          --jq '.assets[] | select(.name|endswith("SHA256SUMS")) | .id')"
gh api "repos/utelle/SQLite3MultipleCiphers/releases/assets/$sums" \
  -H "Accept: application/octet-stream" > "$work/SHA256SUMS"
expected="$(grep " $name\$\| \*$name\$" "$work/SHA256SUMS" | awk '{print $1}')"
actual="$(sha256sum "$work/mc.zip" | awk '{print $1}')"
[ -n "$expected" ] || { echo "no published sum for $name" >&2; exit 1; }
[ "$expected" = "$actual" ] || {
  echo "checksum mismatch for $name" >&2
  echo "  published $expected" >&2
  echo "  fetched   $actual" >&2
  exit 1
}
echo "   $tag  $name  sha256 ok"

unzip -q -o "$work/mc.zip" -d "$work/mc"

echo "── building vendor/libsqlite3-sys ──"
rm -rf "$vendor"
mkdir -p "$(dirname "$vendor")"
cp -r "$src" "$vendor"
chmod -R u+w "$vendor"
cp "$work/mc/sqlite3mc_amalgamation.c" "$vendor/sqlite3/sqlite3.c"
cp "$work/mc/sqlite3.h"                "$vendor/sqlite3/sqlite3.h"
cp "$work/mc/sqlite3ext.h"             "$vendor/sqlite3/sqlite3ext.h"

echo
echo "ready. \`cargo run -p postio-app\` is now Postio on ChaCha20-Poly1305."
