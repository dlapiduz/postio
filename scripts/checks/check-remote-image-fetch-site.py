#!/usr/bin/env python3
"""Hold the composer's one network request to the press that asks for it.

Spec 014 D7 and FR-031: a remote image a draft names is downloaded only when
the person presses Download and attach (`attach_remote_image`), one request a
press, and never on any other gesture. The privacy argument is a call graph:
`postio_session::attaching::fetch_remote_image` is the only function that
makes that request, and its only caller is the host's handler for
`Req::AttachRemoteImage`, which only that command sends.

# The rule

Outside the function's own file and any `tests/` directory, a line calling
``fetch_remote_image(`` must sit inside the `Req::AttachRemoteImage` arm --
within the ``WINDOW`` lines after a line naming it -- and there is at most one
such call in the workspace's sources.

# Exit status

0 clean, 1 a call outside the press's handler or a second one, 2 the check
could not run.

# Usage

    python3 scripts/checks/check-remote-image-fetch-site.py
    python3 scripts/checks/check-remote-image-fetch-site.py --root DIR   # a fixture
"""

from __future__ import annotations

import sys
from pathlib import Path

NEEDLE = "fetch_remote_image("
HANDLER = "Req::AttachRemoteImage"
DEFINITION = "crates/postio-session/src/attaching.rs"
WINDOW = 25


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parents[2]
    args = argv[1:]
    if args[:1] == ["--root"] and len(args) > 1:
        root = Path(args[1])
    elif args:
        print(f"remote-image check could not run: unknown argument {args[0]}", file=sys.stderr)
        return 2
    crates = root / "crates"
    if not crates.is_dir():
        print(f"remote-image check could not run: no {crates}", file=sys.stderr)
        return 2
    problems: list[str] = []
    sites: list[str] = []
    for path in sorted(crates.rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        if rel == DEFINITION or "/tests/" in rel or "/target/" in rel:
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        for number, line in enumerate(lines, start=1):
            code = line.split("//", 1)[0]
            if NEEDLE not in code or "fn fetch_remote_image" in code:
                continue
            before = lines[max(0, number - 1 - WINDOW) : number - 1]
            if not any(HANDLER in earlier for earlier in before):
                problems.append(
                    f"{rel}:{number}: fetch_remote_image is called outside the "
                    f"{HANDLER} handler; only the Download and attach press may "
                    "make that request (spec 014 FR-031)"
                )
            sites.append(f"{rel}:{number}")
    if len(sites) > 1:
        problems.append(
            "fetch_remote_image has more than one caller: " + ", ".join(sites)
            + "; the press's one handler is the only one allowed"
        )
    for problem in problems:
        print(problem)
    if problems:
        return 1
    print(f"remote-image fetch: {len(sites)} call site, in the press's handler")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
