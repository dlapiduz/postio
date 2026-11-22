#!/usr/bin/env bash
# Remove this workspace's own compiled artifacts from `target/` before CI hands it to
# `actions/cache`: they are stale next run,
# while the ~470 third-party crates are worth keeping. A script, not a copy per job, to avoid drift
# (#818).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/drop-workspace-artifacts.sh"
before=$(du -sm target 2>/dev/null | cut -f1 || echo 0)
drop_workspace_artifacts target
echo "cached target: ${before}MB -> $(du -sm target 2>/dev/null | cut -f1 || echo 0)MB"
