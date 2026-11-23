# The queue labels issue-claim.sh and issue-release.sh must agree on: one definition so a third
# queue cannot desync them (#621); `--ready-label` can still name anything.
# issue-release.sh does not know which queue an issue came from, so it strips every label here the
# issue wears; an issue carries at most one.
# `ready-mac` stays (#668): it is about which machine can run the gate (WebKitGTK has no macOS
# build, `macos/` needs Xcode), so it keeps a Linux session off an issue it cannot observe.
# Add a queue the day it needs to survive release. Sourced, not executed: every caller shares one
# `set -euo pipefail`.
READY_LABELS=(ready ready-mac)
