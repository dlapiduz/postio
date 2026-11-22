# Sourced by scripts/install-{nextest,hakari,machete,cargo-deny}.sh.
# The pins install side by side: sessions on different commits share ~/.cargo/bin, and a branch
# bumping a pin made them swap versions until it merged.
# Each version gets ${POSTIO_TOOLS_DIR:-~/.local/share/postio-tools}/<cargo-tool>/<version>/bin/ and
# ~/.cargo/bin/<cargo-tool> becomes `cargo-tool-shim.sh`,
# which reads the pin from the checkout it runs in. `cargo nextest` is looked up on PATH as
# `cargo-nextest`, so callers need no cooperation;
# mise cannot pin nextest and aliases cannot name per-commit versions.
# CI (GITHUB_ACTIONS set) keeps the plain install, with no shim, so ci.yml's cache keyed on the
# install script is unchanged.

PT_LIB=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

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

# True when the right version is in place: `--version`, not existence, since a cached copy at the
# wrong version looks identical on PATH.
pt_have() {
    [ -x "$PT_BIN/$PT_NAME" ] || return 1
    # shellcheck disable=SC2086
    "$PT_BIN/$PT_NAME" $PT_SUB --version 2>/dev/null | grep -qwF "$PT_VERSION"
}

# Make ~/.cargo/bin/<tool> the resolver. Copy, not symlink (a symlink into a worktree dies with
# `issue-release.sh`); write beside and rename.
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

# pt_stage: a fresh root beside the final directory, so the commit is a rename and a failed download
# leaves nothing that looks installed.
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
        if [ ! -d "$PT_ROOT" ]; then
            mv "$PT_STAGE" "$PT_ROOT" 2>/dev/null || [ -d "$PT_ROOT" ]
        fi
    else
        mkdir -p "$PT_BIN"
        mv -f "$PT_STAGE/bin/$PT_NAME" "$PT_BIN/$PT_NAME"
    fi
    pt_install_shim
}

pt_cargo_install() {
    pt_stage
    cargo install "$PT_NAME" --version "$PT_VERSION" --locked --root "$PT_STAGE"
    pt_commit
    echo "$PT_NAME $PT_VERSION installed"
}
