#!/usr/bin/env bash
# Which workspace crates this host cannot build, one per line.
#
# Prints nothing on a host with every system library, which is the ordinary
# case and the one that must stay fast.
#
# # Why this is derived and not a list
#
# `issue-land.sh` used to name `postio-gtk postio-app` inline. That is the
# right *root* set -- they are the crates whose system libraries can be
# missing -- and the wrong answer, because the question a gate needs is
# "what can I not compile", and that is the root set plus everything that
# reaches it:
#
#     glib-sys -> gio-sys -> gio -> glib-build-tools
#                 [build-dependencies] -> postio-gtk
#                 [dev-dependencies]   -> postio-bench
#
# `postio-bench` has no GTK dependency anybody would notice reading its
# manifest -- it dev-depends on `postio-gtk`, so `cargo test -p postio-bench`
# needs WebKit and `cargo test --workspace --exclude postio-gtk` drags the
# whole stack back in through it. A hardcoded pair would have been correct
# on the day it was written and wrong the next time a crate dev-depends on
# the frontend: silently, and only on macOS, which is where nobody is
# looking (#1152).
#
# # How the roots are chosen
#
# By asking `pkg-config`, not `uname`. A Linux box without the -dev packages
# is in exactly the same position as a Mac and would otherwise pass a check
# that named an operating system. #555 made that call; this keeps it.
#
# Usage:
#   scripts/unbuildable-crates.sh          # crate names, one per line
#   scripts/unbuildable-crates.sh --libs   # the missing libraries instead
#
# Exit status: always 0. "Nothing is unbuildable" is an answer, not an error.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The libraries the frontend needs. gtk4 and libadwaita have arm64 bottles;
# webkitgtk has none, and the reader and composer are both WebKit views.
MISSING=""
for lib in gtk4 libadwaita-1 webkitgtk-6.0; do
    pkg-config --exists "$lib" 2>/dev/null || MISSING="${MISSING:+$MISSING }$lib"
done

if [ "${1:-}" = "--libs" ]; then
    printf '%s\n' "$MISSING"
    exit 0
fi

# Every library present: nothing to exclude, and no reason to pay for
# `cargo metadata` to say so.
[ -n "$MISSING" ] || exit 0

# The roots, and then everything that reaches them. `cargo metadata` resolves
# the workspace without compiling anything.
cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c '
import json, sys

meta = json.load(sys.stdin)
members = {p["name"]: p for p in meta["packages"]}

# The crates whose own system libraries are missing.
roots = {"postio-gtk", "postio-app"} & members.keys()

# Everything that depends on one, by any edge -- normal, build or dev. A dev
# edge is what makes postio-bench unbuildable, and it is the edge a reader of
# the manifest is least likely to notice.
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
