#!/usr/bin/env bash
# Is this push a release, and what does the release workflow need to know?
#
# `release.yml` runs on every push to `main` (#1714). A version moves by a
# release pull request (`scripts/release-prepare.sh`), so the push that
# merges one is the only push whose `Cargo.toml` names a version that has no
# tag yet -- and that is the whole test. Every other push answers "nothing to
# build" in the time it takes to read a file and list tags.
#
# A version is released at most once: if its tag exists the answer is no,
# whatever else is true, because a second run would rebuild and re-upload
# over a published release. A version *older* than the newest tag is refused
# too, and said out loud -- that is a mistake in a release PR, not a release.
#
# # The suite
#
# Nothing ships without the full suite. But the nightly runs the identical
# one (`--profile ci-full` through `test-with-flake-retry.sh`, plus the
# doctests), so a green nightly at this *exact* commit is that answer
# already given, and running it again is forty minutes of saying it twice.
# Anything short of the exact commit is not the same answer, and no answer
# from the API is not a green one. `--skip-suite` is honoured only for a
# dispatch, which publishes nothing.
#
# # By hand
#
# `workflow_dispatch` builds every package and publishes nothing, versioned
# `<version>-dev.<sha>` so an artifact from it cannot pass for the release.
#
# Usage: scripts/release-plan.sh [--skip-suite]
#   Run from the repository root with the tags fetched. Reads
#   GITHUB_EVENT_NAME, GITHUB_SHA and GITHUB_REPOSITORY. Prints
#   `key=value` lines for $GITHUB_OUTPUT on stdout -- build, publish,
#   version, tag, prerelease, suite -- and the reasoning on stderr.
set -euo pipefail

SKIP_SUITE=0
for arg in "$@"; do
    case "$arg" in
        --skip-suite) SKIP_SUITE=1 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

EVENT="${GITHUB_EVENT_NAME:-}"
SHA="${GITHUB_SHA:-$(git rev-parse HEAD)}"

# The one `version = "x.y.z"` line: [workspace.package]'s, not rust-version.
VERSION=$(sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"$/\1/p' Cargo.toml | head -n 1)
if [ -z "$VERSION" ]; then
    echo "no workspace version in Cargo.toml" >&2
    exit 1
fi

# x.y.z as one comparable number; each part well under a million.
as_number() {
    echo "$1" | awk -F. '{ printf "%d\n", ($1 * 1000000 + $2) * 1000000 + $3 }'
}

# Empty when there are no tags at all -- a first release. `|| true` because
# grep matching nothing is that case, not a failure, and pipefail would
# otherwise end the script with no answer.
newest_tag() {
    { git tag --list 'v[0-9]*.[0-9]*.[0-9]*' | sed 's/^v//' |
        grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' || true; } |
        while read -r v; do echo "$(as_number "$v") $v"; done |
        sort -n | tail -n 1 | cut -d' ' -f2
}

emit() { printf '%s=%s\n' "$1" "$2"; }

nothing() {
    echo "$1" >&2
    emit build false
    emit publish false
    emit version "$VERSION"
    emit tag ""
    emit prerelease false
    emit suite skip
    exit 0
}

case "$VERSION" in 0.*) PRERELEASE=true ;; *) PRERELEASE=false ;; esac

if [ "$EVENT" = "workflow_dispatch" ]; then
    BUILD_VERSION="${VERSION}-dev.$(git rev-parse --short=8 "$SHA")"
    PUBLISH=false
    TAG=""
    echo "Dispatched by hand: building ${BUILD_VERSION}, publishing nothing." >&2
else
    if git rev-parse -q --verify "refs/tags/v${VERSION}" >/dev/null; then
        nothing "v${VERSION} is already released — nothing to do."
    fi
    NEWEST=$(newest_tag)
    if [ -n "$NEWEST" ] && [ "$(as_number "$VERSION")" -le "$(as_number "$NEWEST")" ]; then
        nothing "::warning::Cargo.toml says ${VERSION}, which is not newer than v${NEWEST} — not releasing. A release PR moves the version forward (scripts/release-prepare.sh)."
    fi
    BUILD_VERSION="$VERSION"
    PUBLISH=true
    TAG="v${VERSION}"
    echo "Releasing ${TAG} (newest tag: ${NEWEST:-none})." >&2
fi

SUITE=run
if [ "$SKIP_SUITE" = 1 ] && [ "$PUBLISH" = false ]; then
    SUITE=skip
    echo "Suite: skipped on request; this run publishes nothing." >&2
else
    green=$(gh api \
        "repos/${GITHUB_REPOSITORY}/actions/workflows/nightly.yml/runs?head_sha=${SHA}&status=success&per_page=1" \
        --jq '.total_count' 2>/dev/null) || green=""
    if [ -n "$green" ] && [ "$green" -gt 0 ] 2>/dev/null; then
        SUITE=skip
        echo "Suite: a green nightly already ran the full suite on ${SHA} — not running it twice." >&2
    else
        echo "Suite: no green nightly on ${SHA} — running it here." >&2
    fi
fi

emit build true
emit publish "$PUBLISH"
emit version "$BUILD_VERSION"
emit tag "$TAG"
emit prerelease "$PRERELEASE"
emit suite "$SUITE"
