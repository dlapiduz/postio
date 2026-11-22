# What release-plan.sh and release-prepare.sh must agree on: how versions order, which tag is
# newest, and the notes when nobody wrote any (#1714).
# Sourced, not executed: every caller shares one `set -euo pipefail`.

release_as_number() {
    echo "$1" | awk -F. '{ printf "%d\n", ($1 * 1000000 + $2) * 1000000 + $3 }'
}

# Every vX.Y.Z tag on origin, not the local list: the workflow checkout has no tags, and an empty
# list once set out to publish v0.4.2 over the
# published one. `ls-remote` fails rather than answering "none" when it cannot ask, which the caller
# must treat as a refusal.
release_tags() {
    git ls-remote --tags --refs origin 'refs/tags/v*' | sed 's#^.*refs/tags/##'
}

# The newest of the tags on stdin, without the v; empty for a first release (`|| true` because grep matching nothing is that case).
release_newest_of() {
    { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true; } | sed 's/^v//' |
        while read -r v; do echo "$(release_as_number "$v") $v"; done |
        sort -n | tail -n 1 | cut -d' ' -f2
}

release_newest_tag() {
    local tags
    tags=$(release_tags) || return 1
    printf '%s\n' "$tags" | release_newest_of
}

release_workspace_version() {
    sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"$/\1/p' "${1:-Cargo.toml}" | head -n 1
}

# Notes for a release nobody wrote notes for: commit subjects since the newest tag; unreadable past
# a few dozen, so docs/releases/<version>.md wins wherever it exists.
release_draft_notes() {
    local since="$1" range
    if [ -n "$since" ]; then range="v${since}..HEAD"; else range="HEAD"; fi
    { git log "$range" --no-merges --format='- %s' | grep -v '^- chore(release):' || true; }
}
