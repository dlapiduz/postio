#!/usr/bin/env python3
"""Self-test for scripts/checks/check-remote-image-fetch-site.py.

The composer's one network request is made only by the Download and attach
press (spec 014 FR-031). Throwaway repositories in a temp dir, one per way
the rule holds or breaks, and an assertion for each. The real repository is
never touched.

Usage: scripts/tests/test-check-remote-image-fetch-site.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-remote-image-fetch-site.py"
FAILURES: list[str] = []

HANDLER = """
match request {
    Req::AttachRemoteImage { draft, href } => {
        let outcome = attaching::fetch_remote_image(&fetcher, &blobs, draft, &href, true);
    }
}
"""


def expect(name: str, files: dict[str, str], ok: bool, *seen: str) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "crates").mkdir()
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run(
            [sys.executable, str(CHECK), "--root", str(root)],
            capture_output=True,
            text=True,
        )
    output = result.stdout + result.stderr
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{output}")
    for text in seen:
        if text not in output:
            FAILURES.append(f"{name}: output lacks {text!r}\n{output}")


expect("the handler's call passes", {"crates/postio-host/src/lib.rs": HANDLER}, True)
expect(
    "the definition and the tests pass",
    {
        "crates/postio-session/src/attaching.rs": "pub fn fetch_remote_image(x) {}",
        "crates/postio-session/tests/suite/remote_image.rs": "fetch_remote_image(&fake);",
        "crates/postio-host/src/lib.rs": HANDLER,
    },
    True,
)
expect(
    "a call on another gesture fails",
    {"crates/postio-widgets/src/composer/mod.rs": "fn autosave() { fetch_remote_image(&f, &b, d, &h, true); }"},
    False,
    "crates/postio-widgets/src/composer/mod.rs:1: fetch_remote_image is called outside",
)
expect(
    "a second call, even in a handler, fails",
    {"crates/postio-host/src/lib.rs": HANDLER + HANDLER},
    False,
    "more than one caller",
)
expect("a comment naming it passes", {"crates/postio-host/src/lib.rs": "// fetch_remote_image( is the press's"}, True)

if FAILURES:
    print("\n\n".join(FAILURES))
    sys.exit(1)
print("check-remote-image-fetch-site: every case behaved")
