#!/usr/bin/env python3
"""Refuse email's own tracking mechanisms unless consent is written down.

Postio's promise is one sentence: *nothing leaves this machine that the user
did not ask for.* Blocking remote images is the visible half and the easy
half. Email carries two other mechanisms that report a real human read the
message, and both are trivial to add by accident because both look like
ordinary features:

  * **Read receipts (MDN).** ``Disposition-Notification-To`` and
    ``Return-Receipt-To`` ask the client to send a message back on open. A
    client that honours them silently has told the sender their address is
    live, that it is monitored, and roughly when it is read.
  * **List-Unsubscribe One-Click.** ``List-Unsubscribe-Post`` invites a POST.
    Sending it confirms to a spammer that the address is real. Legitimate
    senders honour it; the ones worth unsubscribing from harvest it.

Neither exists in Postio today, which is the whole reason this check does. A
guard that fires when nothing is wrong is easy; what is hard is noticing the
day a plausible patch adds ``Disposition-Notification-To`` handling with a
setting that defaults to on. `postio-qhz.2` asked for exactly that guard.

# The rule

Any file under ``crates/*/src`` that mentions one of these mechanisms must
also carry a ``POSTIO-CONSENT:`` line saying how the user asks for it. The
check does not — cannot — verify that the consent path is real. What it
guarantees is that the decision was *made and written down* rather than
arriving inside a diff about something else, which is the failure mode worth
catching: nobody adds a tracker on purpose.

Consent must be per-message and deliberate. Never a setting that defaults on,
never on render, never prefetched. See CLAUDE.md, "Privacy is a feature, not a
setting", and docs/PRODUCT.md §21.

# Exit status

0 clean, 1 a mechanism appeared without consent, 2 the check could not run.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# --- The mechanisms ---------------------------------------------------------
#
# Spelled as they appear in a header name, which is how they appear in code
# that acts on them. Matching is case-insensitive: header names are, and a
# patch writing `disposition-notification-to` should not slip past.

TRACKING: dict[str, str] = {
    "disposition-notification-to": "read receipt (MDN)",
    "return-receipt-to": "read receipt (legacy Return-Receipt-To)",
    "list-unsubscribe-post": "List-Unsubscribe One-Click",
}

# The marker that says a human decided how the user asks for this.
MARKER = "POSTIO-CONSENT:"

# --- Known-benign occurrences ----------------------------------------------
#
# Deliberately tiny, and deliberately per-file rather than a pattern: the
# point of the check is that a *new* occurrence has to be argued for, and an
# allowlist that accepted a glob would quietly accept the next one too.

ALLOWED: dict[str, str] = {
    "crates/postio-model/src/test_corpus.rs": (
        "prose: the corpus registry's one-line description of the newsletter "
        "fixture, which is a description of test mail rather than anything "
        "that acts on a header"
    ),
}


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked_sources() -> list[Path]:
    """Every Rust file under a crate's ``src``, as git sees it.

    Asking git rather than walking the tree keeps the check off build output
    and off another session's scratch files, and means an untracked
    experiment cannot fail somebody else's run.
    """
    try:
        listed = subprocess.run(
            ["git", "ls-files", "crates/*/src/**/*.rs", "crates/*/src/*.rs"],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error

    return [Path(line) for line in listed.stdout.splitlines() if line]


def offences(path: Path) -> list[tuple[int, str, str]]:
    """Every mechanism named in `path`, as ``(line number, name, what)``."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []

    if MARKER in text:
        # Consent is recorded for this file. Whether it is a *good* consent
        # path is a review question, not one a grep can answer.
        return []

    found = []
    for number, line in enumerate(text.splitlines(), start=1):
        lowered = line.lower()
        for mechanism, description in TRACKING.items():
            if mechanism in lowered:
                found.append((number, mechanism, description))
    return found


def main() -> int:
    try:
        sources = tracked_sources()
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    problems: list[str] = []
    for path in sources:
        allowed = ALLOWED.get(path.as_posix())
        for number, mechanism, description in offences(path):
            if allowed:
                continue
            problems.append(f"{path}:{number}: {description} — `{mechanism}`")

    if not problems:
        print(f"no-silent-tracking check passed ({len(sources)} files).")
        return 0

    print("no-silent-tracking check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} occurrence(s).\n\n"
        "These are email's own tracking mechanisms. Sending one tells the\n"
        "sender that a real person read the message, and Postio's promise is\n"
        "that nothing leaves the machine the user did not ask for.\n\n"
        "If you are adding one deliberately, say how the user asks for it, in\n"
        "the file, on a line like:\n\n"
        f"    // {MARKER} sent only from the reader's `Send receipt` button,\n"
        "    // per message, never from a setting and never on render.\n\n"
        "Consent must be per-message and explicit. Never a default-on\n"
        "setting, never on render, never prefetched. Prefer the `mailto:`\n"
        "form of List-Unsubscribe, which goes through the send path the user\n"
        "can see. See CLAUDE.md, \"Privacy is a feature, not a setting\".",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
