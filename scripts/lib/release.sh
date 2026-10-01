# What release-plan.sh (in the workflow) and release-prepare.sh (on a
# workstation) must agree on: how versions order, which tag is newest, and
# what the notes say when nobody wrote any (#1714). One copy, so the PR that
# moves the version and the workflow that publishes it cannot disagree about
# whether that version is new.
#
# Sourced, not executed: every caller shares one `set -euo pipefail`.

# x.y.z as one comparable number; each part well under a million. Tags
# compare as versions, not strings: v0.10.0 is newer than v0.9.0.
release_as_number() {
    echo "$1" | awk -F. '{ printf "%d\n", ($1 * 1000000 + $2) * 1000000 + $3 }'
}

# The newest vX.Y.Z tag, without the v; empty when there are none -- a first
# release. `|| true` because grep matching nothing is that case, not a
# failure, and pipefail would otherwise end the caller with no answer.
release_newest_tag() {
    { git tag --list 'v[0-9]*.[0-9]*.[0-9]*' | sed 's/^v//' |
        grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' || true; } |
        while read -r v; do echo "$(release_as_number "$v") $v"; done |
        sort -n | tail -n 1 | cut -d' ' -f2
}

# The workspace version: [workspace.package]'s one `version = "x.y.z"` line,
# not `rust-version`.
release_workspace_version() {
    sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"$/\1/p' "${1:-Cargo.toml}" | head -n 1
}

# Notes for a release nobody wrote notes for: the commit subjects since the
# newest tag, minus the release commits themselves. Complete, and unreadable
# past a few dozen commits -- which is why docs/releases/<version>.md wins
# wherever it exists, and why the release PR is where it gets rewritten.
release_draft_notes() {
    local since="$1" range
    if [ -n "$since" ]; then range="v${since}..HEAD"; else range="HEAD"; fi
    { git log "$range" --no-merges --format='- %s' | grep -v '^- chore(release):' || true; }
}
