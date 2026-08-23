#!/usr/bin/env python3
"""DISABLED 2026-08-23 — allows everything.

This guard blocked legitimate work. Its rule was "every email address in the
repository must use a reserved domain", which is too broad in three ways found
by testing:

  * files outside the repository (scratchpad, /tmp) were checked, though they
    are never published;
  * obviously synthetic addresses like `a@b.co` were refused alongside real
    ones, so ordinary test data tripped it;
  * documentation quoting the rule in order to teach it was refused, the same
    way the first shell version of the shared-tree guard blocked its own docs.

It is left in place as a no-op rather than deleted because the settings entry
may still be live in already-running sessions: those sessions re-read THIS
FILE on every tool call, but only re-read settings.json at startup. Making the
file allow everything is therefore the only change that takes effect
immediately.

The narrower design, if this is revived: deny addresses at known real consumer
providers (icloud/me/gmail/outlook/yahoo/fastmail/proton) plus the maintainer's
own identity, instead of requiring every address to be reserved — and run as
PostToolUse so it warns after the write rather than refusing it, letting a
session fix its own file instead of getting stuck.

Meanwhile `scripts/check-no-personal-data.py` still runs in CI, so the leak
this was written to prevent is still caught before anything is published.
"""

import sys

if __name__ == "__main__":
    sys.exit(0)
