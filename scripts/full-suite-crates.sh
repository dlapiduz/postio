#!/usr/bin/env bash
# Which of the crates you changed get their integration suites run too.
#
# Reads crate names, one per line, on stdin; prints the subset. `--slow`
# prints the exception list instead, which is what its self-test checks
# against the tree.
#
# # Why the sanity tier is not enough for some changes
#
# `issue-land.sh` runs the whole workspace's *unit* tests, and
# `cargo check --workspace --all-targets` proves everything compiles. Neither
# can see a whole class of failure, because several tests **enumerate** a
# vocabulary rather than compiling against it:
#
#   postio-core/tests/platform_bindings.rs   the golden Linux binding table
#   postio-core/tests/command_registry.rs    CONFIG_BINDINGS vs the registry
#   postio-core/tests/keybindings_doc.rs     docs/keybindings.md
#   postio-config/tests/schema.rs            [keys], [ui]
#   postio-config/tests/config_doc.rs        docs/config.md
#
# Adding one `CommandId` touches six or seven places and the compiler checks
# two of them, so the branch builds, lands, and fails CI ten minutes later on
# an assertion about a table. #1003 paid that twice in one session, on two
# *different* tables (#1047).
#
# # Why this is a deny-list, and that is the whole design
#
# The obvious shape is a list of crates worth testing. It is the wrong way
# round: it fails **silently**. A golden table added to a crate nobody put on
# the list goes unchecked, and nothing says so -- the landing is green and
# fast, which is exactly how this class of bug already reaches CI.
#
# So the rule is derived from the diff -- every crate you changed -- minus the
# few whose suites cost minutes. A stale exception list fails the other way:
# a crate that has grown slow makes a landing slower, which whoever is waiting
# notices immediately.
#
# # The exception, measured
#
# Integration-test execution per crate, from CI run 33856312398 (3,578 tests):
#
#   postio-gtk      283s      postio-sync      17s     postio-model    5.3s
#   postio-app      208s      postio-account   15s     postio-ffi      3.5s
#   postio-storage   76s      postio-index     13s     postio-search   2.1s
#   postio-runtime   45s      postio-config    12s     postio-body     1.7s
#                             postio-core     6.3s     postio-smtp     0.9s
#
# The gap is wide and lands between `postio-runtime` and `postio-sync`: four
# crates cost 45s and up, and everything else is under twenty. Those four are
# the exception; the rest a landing can afford, and CI still runs all of them
# on the pull request either way.
set -euo pipefail

# Crates whose integration suites are minutes rather than seconds. See above
# for the measurements and for why this list -- and not its inverse -- is the
# one that is maintained.
SLOW="postio-app postio-gtk postio-runtime postio-storage"

if [ "${1:-}" = "--slow" ]; then
    printf '%s\n' $SLOW
    exit 0
fi

while read -r crate; do
    [ -n "$crate" ] || continue
    skip=0
    for slow in $SLOW; do
        [ "$crate" = "$slow" ] && skip=1
    done
    [ "$skip" = 1 ] || printf '%s\n' "$crate"
done
