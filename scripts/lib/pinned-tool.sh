# Sourced by scripts/install-{nextest,hakari,machete,cargo-deny}.sh.
#
# # Why the pins install side by side
#
# Many sessions share one machine and one ~/.cargo/bin, each in a worktree
# on a different commit. A branch that bumps a pin installs the new version
# over the old binary, every session still on main's pin reinstalls the old
# one, and they swap until the bump merges (nextest 0.9.143 <-> 0.9.152).
#
# So each version gets its own directory,
#     ${POSTIO_TOOLS_DIR:-~/.local/share/postio-tools}/<cargo-tool>/<version>/bin/
# and ~/.cargo/bin/<cargo-tool> becomes `cargo-tool-shim.sh`, which reads the
# pin from the checkout it is run in and execs that version.
#
# Alternatives weighed:
#   * Versioned names only (cargo-nextest-0.9.152): `cargo nextest` still
#     finds the plain name, so nothing says which version it runs.
#   * A PATH entry or env var per worktree: every session, hook and editor
#     would have to set it; cargo finds subcommands only by PATH.
#   * mise or a `.cargo/config.toml` alias: mise cannot pin nextest
#     (mise.toml says why), and aliases cannot name per-commit versions.
#   * The shim (chosen): `cargo nextest` is already looked up on PATH as
#     `cargo-nextest`, so it needs no cooperation from callers; the pin
#     stays in the one script that holds it; a version missing from the
#     store is installed on first use.
#
# CI (GITHUB_ACTIONS set) keeps the plain install: one commit per runner,
# so it installs straight into ~/.cargo/bin and ci.yml's cache of that file,
# keyed on the install script's hash, is unchanged. No shim there.
#
# Checksums and `--locked` are exactly what each script did before; this
# file only decides where the result lands.

PT_LIB=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

# pt_init <cargo-tool> <version> [<version-subcommand>]
# Sets PT_BIN (where the binary lands), PT_NAME and PT_VERSION.
pt_init() {
    PT_NAME="$1"
    PT_VERSION="$2"
    PT_SUB="${3:-}"
    PT_CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        PT_ROOT=""
        PT_BIN="$PT_CARGO_BIN"
    else
        PT_ROOT="${POSTIO_TOOLS_DIR:-$HOME/.local/share/postio-tools}/$PT_NAME/$PT_VERSION"
        PT_BIN="$PT_ROOT/bin"
    fi
}

# True when the right version is already in place. `--version` rather than
# existence: a cached or hand-installed copy at the wrong version is the
# case that matters, and it looks identical on PATH.
pt_have() {
    [ -x "$PT_BIN/$PT_NAME" ] || return 1
    # shellcheck disable=SC2086
    "$PT_BIN/$PT_NAME" $PT_SUB --version 2>/dev/null | grep -qwF "$PT_VERSION"
}

# Make ~/.cargo/bin/<tool> the resolver. Copy, not symlink: a symlink into a
# worktree dies with `issue-release.sh`. Write beside and rename, so a
# running shim never reads a half-copied script.
pt_install_shim() {
    [ -z "$PT_ROOT" ] && return 0
    local target="$PT_CARGO_BIN/$PT_NAME"
    mkdir -p "$PT_CARGO_BIN"
    if [ -f "$target" ] && cmp -s "$PT_LIB/cargo-tool-shim.sh" "$target"; then
        return 0
    fi
    cp "$PT_LIB/cargo-tool-shim.sh" "$target.tmp.$$"
    chmod 755 "$target.tmp.$$"
    mv -f "$target.tmp.$$" "$target"
    echo "installed $target (resolves the pin of the checkout it runs in)"
}

# pt_stage: a fresh root to install into; the tool lands in $PT_STAGE/bin.
# Beside the final directory so the commit is a rename, and a failed
# download leaves nothing that looks installed.
pt_stage() {
    if [ -n "$PT_ROOT" ]; then
        mkdir -p "$(dirname "$PT_ROOT")"
        PT_STAGE=$(mktemp -d "$(dirname "$PT_ROOT")/.stage-XXXXXX")
    else
        PT_STAGE=$(mktemp -d)
    fi
    mkdir -p "$PT_STAGE/bin"
    trap 'rm -rf "$PT_STAGE"' EXIT
}

pt_commit() {
    if [ -n "$PT_ROOT" ]; then
        # A concurrent session may have finished first; theirs is the same bytes.
        if [ ! -d "$PT_ROOT" ]; then
            mv "$PT_STAGE" "$PT_ROOT" 2>/dev/null || [ -d "$PT_ROOT" ]
        fi
    else
        mkdir -p "$PT_BIN"
        mv -f "$PT_STAGE/bin/$PT_NAME" "$PT_BIN/$PT_NAME"
    fi
    pt_install_shim
}

# pt_cargo_install: for tools published as source only.
pt_cargo_install() {
    pt_stage
    cargo install "$PT_NAME" --version "$PT_VERSION" --locked --root "$PT_STAGE"
    pt_commit
    echo "$PT_NAME $PT_VERSION installed"
}
