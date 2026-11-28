#!/usr/bin/env bash
# Which workspace crates this host cannot build, one per line; nothing on a host with every system
# library.
# Derived, not a list (#1152): the roots (the GTK crates, whose system libraries can be missing)
# plus everything that reaches them by any edge;
# `postio-bench` dev-depends on the desktop app, so it needs WebKit too. The roots are chosen by
# `pkg-config`, not `uname`: a Linux box without the
# -dev packages is in the same position as a Mac (#555).
#
# Usage:
#   scripts/unbuildable-crates.sh          # crate names, one per line
#   scripts/unbuildable-crates.sh --libs   # the missing libraries instead
#
# Exit status: always 0. "Nothing is unbuildable" is an answer, not an error.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

MISSING=""
for lib in gtk4 libadwaita-1 webkitgtk-6.0; do
    pkg-config --exists "$lib" 2>/dev/null || MISSING="${MISSING:+$MISSING }$lib"
done

if [ "${1:-}" = "--libs" ]; then
    printf '%s\n' "$MISSING"
    exit 0
fi

[ -n "$MISSING" ] || exit 0

cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c '
import json, sys

meta = json.load(sys.stdin)
members = {p["name"]: p for p in meta["packages"]}

roots = {"postio-widgets", "postio-gtk"} & members.keys()

reachable = set(roots)
changed = True
while changed:
    changed = False
    for name, package in members.items():
        if name in reachable:
            continue
        for dependency in package["dependencies"]:
            if dependency["name"] in reachable:
                reachable.add(name)
                changed = True
                break

for name in sorted(reachable):
    print(name)
'
