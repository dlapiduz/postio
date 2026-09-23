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
# # `ready-mac` stays (#668)
#
# The question was whether it still earns its keep once `macos/` is on `main`
# and the crates under it are ordinary crates. It does, and for the reason it
# was created (#552) rather than for the one it is easy to assume: it is not
# about which crates compile, it is about **which machine can run the gate**.
#
# `postio-gtk` and `postio-app` need WebKitGTK, which has no macOS build at
# all, and `macos/` needs Xcode and a window server. Neither can be worked
# from the other host, sessions run on several machines, and the claim locks
# are per-machine -- so the label is the only thing keeping a Linux session
# off an issue whose acceptance criteria it cannot observe. Retiring it would
# not merge the queues; it would let a session claim work, build nothing, and
# find out at the landing gate.
#
# The *branch* is a separate question and is not settled here.
# `feature/macos` is still open (#1306, #668) because the frontend is not at
# parity with the GTK build yet; when it merges, macOS work becomes ordinary
# work on `main`, claimed from this second queue rather than cut from a
# second base. The label survives that either way, which is why it can be
# answered now.
#
# Sourced, not executed: every caller shares one `set -euo pipefail`.
READY_LABELS=(ready ready-mac)
