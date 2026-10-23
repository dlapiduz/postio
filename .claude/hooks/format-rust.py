#!/usr/bin/env python3
"""After an Edit or Write to a `.rs` file in a private worktree, format it.

The guard's most-hit refusals after `git add -A` were formatting: a
crate-wide `cargo fmt`, or a `rustfmt` fed an unscoped file list, either
of which writes to files the session never touched. Both were attempts to
do one small thing -- format what I just edited -- and this does that
thing, so nobody has to type it.

Scope is deliberately one file:

- **Only in a private worktree** (under `POSTIO_WORKTREES`, default
  `~/src/postio-worktrees`), never the shared checkout, where the file may
  be someone else's work in progress.
- **Only the file edited.** It goes through rustfmt's stdin, which formats
  that text and nothing else; `rustfmt <file>` would also rewrite every
  out-of-line child module the file declares.
- **Never `target/` or `vendor/`**, and never a file rustfmt cannot parse --
  mid-edit, that is normal, and the compiler says it better.

When it changes the file it tells the session (`additionalContext`), since
the text on disk no longer matches what the session last wrote and an
`old_string` copied from memory would miss. It never fails a tool call.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

EDITION = "2024"
NOT_OURS = {"target", "vendor"}


def in_private_worktree(path: Path) -> bool:
    root = Path(os.environ.get("POSTIO_WORKTREES") or Path.home() / "src/postio-worktrees")
    try:
        parts = path.resolve().relative_to(root.resolve()).parts
    except ValueError:
        return False
    # <root>/<worktree>/<path within it>, and not under a directory we do not own.
    return len(parts) >= 2 and not NOT_OURS.intersection(parts[1:-1])


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except ValueError:
        return 0
    raw = (payload.get("tool_input") or {}).get("file_path") or ""
    path = Path(raw)
    if path.suffix != ".rs" or not path.is_file() or not in_private_worktree(path):
        return 0
    try:
        before = path.read_bytes()
        env = {k: v for k, v in os.environ.items() if k != "RUSTUP_TOOLCHAIN"}
        # From the file's directory, so rustfmt finds the tree's config and
        # rustup the tree's pinned toolchain.
        result = subprocess.run(
            ["rustfmt", "--edition", EDITION, "--emit", "stdout"],
            input=before, capture_output=True, cwd=path.parent, env=env, timeout=30,
        )
        if result.returncode != 0 or not result.stdout or result.stdout == before:
            return 0
        path.write_bytes(result.stdout)
    except (OSError, subprocess.SubprocessError):
        return 0
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PostToolUse",
        "additionalContext": f"rustfmt reformatted {path} after this edit. Re-read it "
                             "before editing it again; the text on disk has changed.",
    }}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
