# Refuses to run against a `gh` too old for this workflow, in one sentence.
#
# `scripts/issue-claim.sh` reads `gh issue list --json ...,blockedBy,...`.
# `blockedBy` shipped in `gh` 2.94.0 (cli/cli#13057, "Issues 2.0: issue
# types, sub-issues, and relationships"). Before that release `gh` rejects
# the field outright -- it writes an "Unknown JSON field" complaint to
# stderr and nothing to stdout -- so the `python3 -c` piped after it tries
# to parse an empty stream as JSON and dies with `JSONDecodeError`, a
# traceback whose text has nothing to do with the actual problem. #558.
#
# `mise.toml` already pins `gh` above this floor, but mise is optional (see
# that file's own header) -- a session where it is not active gets whatever
# `gh` is already on `$PATH`, which is exactly the class of gap
# `RUSTUP_TOOLCHAIN` leaves for the Rust pin (docs/engineering-notes.md).
# This is the runtime backstop every `scripts/issue-*.sh` sources, so a gap
# like that fails in one sentence here instead of as a stack trace three
# layers downstream.
#
# Sourced, not executed: every caller shares one `set -euo pipefail`, and
# `exit` from a sourced script ends the caller's process the same way a
# check failing partway through the caller's own body would.

REQUIRE_GH_VERSION="2.94.0"

# Numeric MAJOR.MINOR.PATCH comparison, not `sort -V`: BSD `sort` (macOS)
# has no `-V`, and this exact class of GNU-only assumption already broke
# `issue-claim.sh` once on that platform (#559's `slug()`).
_require_gh_version_at_least() {
    local have="$1" want="$2"
    local IFS=.
    # shellcheck disable=SC2206 # word-splitting on IFS=. is the point.
    local -a have_parts=($have) want_parts=($want)
    local i
    for i in 0 1 2; do
        local h="${have_parts[i]:-0}" w="${want_parts[i]:-0}"
        if [ "$h" -gt "$w" ]; then return 0; fi
        if [ "$h" -lt "$w" ]; then return 1; fi
    done
    return 0
}

_require_gh_found_version=$(gh --version 2>/dev/null | awk 'NR==1{print $3}')

if [ -z "${_require_gh_found_version:-}" ]; then
    echo "error: \`gh\` (the GitHub CLI) was not found, or did not report a version." >&2
    echo "This workflow needs gh $REQUIRE_GH_VERSION or newer: https://cli.github.com" >&2
    exit 1
fi

if ! _require_gh_version_at_least "$_require_gh_found_version" "$REQUIRE_GH_VERSION"; then
    echo "error: gh $_require_gh_found_version is too old for this workflow (needs $REQUIRE_GH_VERSION or newer)." >&2
    echo "\`--json blockedBy\` shipped in gh 2.94.0; an older gh fails with an unrelated-looking traceback instead of this message (#558)." >&2
    echo "Upgrade: https://cli.github.com -- or \`mise install\` if you use mise, which already pins a compatible version in mise.toml." >&2
    exit 1
fi

unset _require_gh_found_version
