#!/usr/bin/env bash
# Is this push a release, and what does the release workflow need to know? `release.yml` runs on
# every push to `main` (#1714); only the push that
# merges a release PR names a version with no tag yet, so every other push answers "nothing to
# build".
# A version is released at most once, and one older than the newest tag is refused out loud (a
# mistake in a release PR).
# Nothing ships without the full suite, but a green nightly at this exact commit is that answer
# already; anything short of the exact commit, or no API answer, is not green.
# `--skip-suite` is honoured only for a dispatch, which publishes nothing and builds every package
# versioned `<version>-dev.<sha>`.
#
# Usage: scripts/release-plan.sh [--skip-suite]
#   Run from the repository root; asks origin for its tags. Reads
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

source "$(dirname "${BASH_SOURCE[0]}")/lib/release.sh"

VERSION=$(release_workspace_version)
if [ -z "$VERSION" ]; then
    echo "no workspace version in Cargo.toml" >&2
    exit 1
fi

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
    # What origin has released (see `release_tags`); not knowing is a refusal, since the answer that
    # publishes must be proven.
    if ! TAGS=$(release_tags); then
        echo "::error::Could not list origin's tags, so cannot tell whether v${VERSION} is already out. Not releasing." >&2
        exit 1
    fi
    if printf '%s\n' "$TAGS" | grep -qx "v${VERSION}"; then
        nothing "v${VERSION} is already released — nothing to do."
    fi
    NEWEST=$(printf '%s\n' "$TAGS" | release_newest_of)
    if [ -n "$NEWEST" ] && [ "$(release_as_number "$VERSION")" -le "$(release_as_number "$NEWEST")" ]; then
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
