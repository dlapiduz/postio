#!/usr/bin/env python3
"""Fail if personal data leaks into the repository.

Postio is open source and its test fixtures describe mailboxes, so it is
unusually easy for a maintainer's own name, address or provider account to
end up committed. Two rules:

1. Email addresses in the repository must use a reserved domain (RFC 2606 /
   RFC 6761): example.com/net/org, or the .test, .invalid, .example and
   .localhost TLDs. Hostnames such as imap.mail.me.com are not addresses and
   are unaffected.

2. A denylist of real names must not appear in any tracked file. Sources, in
   order: the POSTIO_DENY_NAMES environment variable (comma separated), then
   this checkout's git config. Never hard-coded, so this file does not have to
   name a real person in order to protect them.

Output is REDACTED by default: it reports the location and the rule, never
the offending value. CI logs on a public repository are public, so a check
that printed what it found would publish exactly what it exists to protect.
Pass --reveal locally to see the values while fixing them.

In CI, set POSTIO_DENY_NAMES from a repository secret; a runner's git config
is the bot's, not a maintainer's, so the git fallback no-ops there. GitHub
masks secret values in logs, which is a second layer under the redaction.

Run: python3 scripts/check-no-personal-data.py [--reveal]
"""

from __future__ import annotations

import os
import re
import subprocess
import sys

RESERVED = re.compile(
    r"@(?:[A-Za-z0-9-]+\.)*(?:example\.(?:com|net|org)"
    r"|(?:[A-Za-z0-9-]+\.)?(?:test|invalid|example|localhost))$",
    re.IGNORECASE,
)
ADDRESS = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")

# Files whose whole point is to name what is forbidden, or which carry
# legitimate upstream addresses.
SKIP_PATHS = (
    "scripts/check-no-personal-data.py",
    # The copyright line names the holder on purpose.
    "LICENSE",
    "Design/",
    "crates/postio-gtk/data/fonts/",
    ".beads/",
)
# Upstream/maintainer addresses that belong in a licence or manifest.
ALLOW_EXACT = {
    "noreply@anthropic.com",
}


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout
    return [p for p in out.splitlines() if not p.startswith(SKIP_PATHS)]


def denied_names() -> list[tuple[str, str]]:
    """Real names that must not appear. Never printed, only matched."""
    env = os.environ.get("POSTIO_DENY_NAMES", "")
    if env.strip():
        return [
            ("POSTIO_DENY_NAMES", v.strip())
            for v in env.split(",")
            if v.strip()
        ]

    ident = []
    for key in ("user.name", "user.email"):
        r = subprocess.run(
            ["git", "config", key], capture_output=True, text=True
        )
        value = r.stdout.strip()
        # A noreply forwarding address is deliberate privacy, not a leak.
        if value and not value.endswith("users.noreply.github.com"):
            ident.append((f"git {key}", value))
    return ident


def main() -> int:
    reveal = "--reveal" in sys.argv
    failures: list[str] = []
    identity = denied_names()

    for path in tracked_files():
        try:
            text = open(path, encoding="utf-8", errors="ignore").read()
        except OSError:
            continue

        for num, line in enumerate(text.splitlines(), 1):
            for address in ADDRESS.findall(line):
                if address in ALLOW_EXACT or RESERVED.search(address):
                    continue
                shown = f" {address!r}" if reveal else ""
                failures.append(
                    f"{path}:{num}: email address on a non-reserved "
                    f"domain{shown}"
                )
            for source, value in identity:
                if value in line:
                    # Never echo the value; naming the source is enough.
                    failures.append(
                        f"{path}:{num}: matches a denied real name "
                        f"(source: {source})"
                    )

    if failures:
        print("personal-data check FAILED\n")
        for f in failures:
            print(f)
        print(f"\n{len(failures)} problem(s).")
        print(
            "\nPostio is open source and its fixtures describe mailboxes.\n"
            "Use a reserved domain and invent the people:\n"
            "    Ada Lovelace <ada@example.com>\n"
            "\nValues are redacted: this output is public in CI. Run\n"
            "    python3 scripts/check-no-personal-data.py --reveal\n"
            "locally to see them."
        )
        return 1

    print("personal-data check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
