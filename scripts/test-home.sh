#!/usr/bin/env bash
# Gives a test run its own XDG home under target/, so no test reads or writes the person's config,
# store, state or downloads. nextest runs it as a setup script (.config/nextest.toml); `--export`
# prints the same for `eval` (scripts/test-fast.sh).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
base="${CARGO_TARGET_DIR:-$PWD/target}/test-home"
mkdir -p "$base"
# Nothing reads a run's home back, so a day is plenty.
find "$base" -mindepth 1 -maxdepth 1 -name 'run.*' -mmin +1440 -exec rm -rf {} + 2>/dev/null || true
run=$(mktemp -d "$base/run.XXXXXX")

# Empty overrides count as unset (postio_config::paths), which `NEXTEST_ENV` cannot do.
vars=(
    "XDG_CONFIG_HOME=$run/config"
    "XDG_DATA_HOME=$run/data"
    "XDG_STATE_HOME=$run/state"
    "XDG_DOWNLOAD_DIR=$run/downloads"
    "XDG_CACHE_HOME=$base/cache"
    "POSTIO_CONFIG="
    "POSTIO_STORE="
    "POSTIO_EXPORT_DIR="
)

if [ "${1:-}" = "--export" ]; then
    for var in "${vars[@]}"; do
        printf 'export %s=%q\n' "${var%%=*}" "${var#*=}"
    done
elif [ -n "${NEXTEST_ENV:-}" ]; then
    printf '%s\n' "${vars[@]}" >>"$NEXTEST_ENV"
else
    echo "usage: run by cargo-nextest as a setup script, or: eval \"\$(scripts/test-home.sh --export)\"" >&2
    exit 2
fi
