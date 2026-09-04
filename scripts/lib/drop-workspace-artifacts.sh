# Drop the workspace's own build products from a target directory and keep
# every dependency's. Sourced; defines drop_workspace_artifacts <target-dir>.
#
# Two callers, one reason. CI caches a target/ between runs and our crates
# are what every run rebuilds anyway (ci-drop-workspace-artifacts.sh). And a
# worktree that was *moved* (a reused claim) or whose target/ was *copied*
# from a sibling (a seeded claim) holds our crates compiled with the old
# tree's absolute path baked in -- `env!("CARGO_MANIFEST_DIR")` is used in
# fourteen files -- and cargo does not notice a directory move: a two-line
# crate moved and re-tested ran the stale binary and failed on the old path,
# and so did postio-session's crate-list test on the first reused landing
# (#1102). Dropping the fingerprints is what makes cargo rebuild ours; the
# ~470 dependencies carry no such path and stay.
#
# Matched by name prefix, which is the workspace's own convention: every
# member is `postio-*`, and rustc spells that `postio_*` in deps/.
drop_workspace_artifacts() { # <target-dir>
    local target="$1"
    [ -d "$target" ] || return 0
    rm -rf "$target"/*/incremental
    rm -f  "$target"/*/deps/libpostio_* \
           "$target"/*/deps/postio_*
    rm -rf "$target"/*/.fingerprint/postio-* \
           "$target"/*/build/postio-*
}
