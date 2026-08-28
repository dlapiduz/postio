# The queue labels issue-claim.sh and issue-release.sh must agree on.
#
# One definition so a third queue cannot silently desync the two the way
# `ready-mac` (#552) did with `ready` itself (#621): issue-claim.sh's
# default read is `${READY_LABELS[0]}`, and its `--ready-label` flag can
# still name anything -- that flexibility is deliberate, a one-off queue
# nobody has bureaucratized yet should still be claimable without editing
# this file. What this guards is the other end: issue-release.sh's
# post-land cleanup does not know which queue an issue came from and must
# not have to guess, so it strips every label named here that the issue
# actually wears -- safe, because an issue carries at most one of them.
#
# Add a queue here the day it needs to survive release, not before -- a
# `--ready-label` used once for a one-off does not need an entry.
#
# Sourced, not executed: every caller shares one `set -euo pipefail`.
READY_LABELS=(ready ready-mac)
