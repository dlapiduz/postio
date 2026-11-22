# Refuses to run against a `gh` too old for this workflow, in one sentence: `gh issue list --json
# blockedBy` needs gh 2.94.0, and older
# versions leave an empty stream that dies as a `JSONDecodeError` (#558). `mise.toml` pins gh but
# mise is optional, so this is the runtime
# backstop every `scripts/issue-*.sh` sources.
# Sourced, not executed: `exit` ends the caller's process.

REQUIRE_GH_VERSION="2.94.0"

# Numeric comparison, not `sort -V`: BSD sort has no `-V` (#559).
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
