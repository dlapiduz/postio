#!/usr/bin/env python3
"""Fail if personal data leaks into the repository.

Postio is open source and its test fixtures describe mailboxes, so it is
unusually easy for a maintainer's own name, address or provider account to
end up committed. Two rules:

1. Email addresses in the repository must use a reserved domain (RFC 2606 /
   RFC 6761): example.com/net/org, or the .test, .invalid, .example and
   .localhost TLDs. Hostnames such as imap.mail.me.com are not addresses and
   are unaffected.

2. The identity configured in this checkout's git config must not appear in
   any tracked file. Read at run time rather than hard-coded, so this file
   never has to name a real person to protect them.

Run: python3 scripts/check-no-personal-data.py
"""

from __future__ import annotations

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


def git_identity() -> list[tuple[str, str]]:
    ident = []
    for key in ("user.name", "user.email"):
        r = subprocess.run(
            ["git", "config", key], capture_output=True, text=True
        )
        value = r.stdout.strip()
        # A noreply forwarding address is deliberate privacy, not a leak.
        if value and not value.endswith("users.noreply.github.com"):
            ident.append((key, value))
    return ident


def main() -> int:
    failures: list[str] = []
    identity = git_identity()

    for path in tracked_files():
        try:
            text = open(path, encoding="utf-8", errors="ignore").read()
        except OSError:
            continue

        for num, line in enumerate(text.splitlines(), 1):
            for address in ADDRESS.findall(line):
                if address in ALLOW_EXACT or RESERVED.search(address):
                    continue
                failures.append(
                    f"{path}:{num}: non-reserved email address {address!r}\n"
                    f"    Use a reserved domain (example.com, .test, .invalid)."
                )
            for key, value in identity:
                if value in line:
                    failures.append(
                        f"{path}:{num}: contains this checkout's git {key}\n"
                        f"    Fixtures must not use a real maintainer's identity."
                    )

    if failures:
        print("personal-data check FAILED\n")
        for f in failures:
            print(f)
        print(f"\n{len(failures)} problem(s).")
        print(
            "\nPostio is open source and its fixtures describe mailboxes.\n"
            "Invent the data: 'Ada Lovelace <ada@example.com>'."
        )
        return 1

    print("personal-data check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
