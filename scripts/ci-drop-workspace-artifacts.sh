#!/usr/bin/env bash
# Remove this workspace's own compiled artifacts from `target/` before a CI job
# hands it to `actions/cache`.
#
# Our twenty crates are the one part of `target/` that is guaranteed stale next
# run: their sources change on every branch, so cargo rebuilds them regardless
# and caching them only pays to upload and download bytes nobody uses. The ~470
# third-party crates behind them are the part worth keeping, and they are the
# bulk of it.
#
# A script rather than a copy in each job, because there are two jobs saving a
# cache now (clippy and test, one each -- see .github/actions/rust-workspace)
# and this repository has been bitten by "three copies of forty lines is three
# chances to drift" before (#818).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/drop-workspace-artifacts.sh"
before=$(du -sm target 2>/dev/null | cut -f1 || echo 0)
drop_workspace_artifacts target
echo "cached target: ${before}MB -> $(du -sm target 2>/dev/null | cut -f1 || echo 0)MB"
