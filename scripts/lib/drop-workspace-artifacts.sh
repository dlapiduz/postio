# Drop the workspace's own build products from a target directory and keep every dependency's.
# Sourced; defines drop_workspace_artifacts <target-dir>. A moved worktree or a copied target/ holds
# our crates with the old absolute path baked in (`env!("CARGO_MANIFEST_DIR")`), and cargo does not
# notice a move (#1102); the ~470 dependencies carry no such path. Matched by the `postio-*` name
# prefix: under build/postio-*/ since Cargo 1.100, and the deps/ lines catch what an older cargo
# left.
drop_workspace_artifacts() { # <target-dir>
    local target="$1"
    [ -d "$target" ] || return 0
    rm -rf "$target"/*/incremental
    rm -f  "$target"/*/deps/libpostio_* \
           "$target"/*/deps/postio_*
    rm -rf "$target"/*/.fingerprint/postio-* \
           "$target"/*/build/postio-*
    # And the copies cargo lifts out of deps/ to the top of each profile: the Mac app's static
    # library is hundreds of megabytes.
    find "$target" -mindepth 2 -maxdepth 2 -type f \
        \( -name 'libpostio_*' -o -name 'postio' -o -name 'postio-*' \) -delete
}

# Drop SwiftPM's build directory from a moved worktree (#1208): it bakes the tree's absolute path
# into every precompiled module, and the next `swift build` fails naming SwiftShims, which reads as
# a broken toolchain.
drop_swift_build() { # <worktree>
    rm -rf "$1/macos/.build"
}
