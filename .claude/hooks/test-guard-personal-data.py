#!/usr/bin/env python3
"""Prove the personal-data guard denies what it must and allows what it must."""
import json
import os
import subprocess
import sys

HOOK = "/home/user/src/postio/.claude/hooks/guard-personal-data.py"
REPO = "/home/user/src/postio"
FIXTURE = f"{REPO}/crates/postio-model/tests/corpus/x.eml"

# (tool, file_path, text, expected)
CASES = [
    # --- must deny -------------------------------------------------------
    ("Write", FIXTURE, "From: Ada <ada@example.com>", "deny"),
    ("Write", FIXTURE, "From: someone@example.net", "deny"),
    ("Edit", f"{REPO}/crates/postio-imap/src/x.rs",
     'let a = "user@example.com";', "deny"),
    ("Write", f"{REPO}/crates/postio-config/src/y.rs",
     '/// e.g. person@fastmail.com', "deny"),
    ("Write", FIXTURE, "From: Diego Lapiduz <ada@example.com>", "deny"),

    # --- must allow ------------------------------------------------------
    ("Write", FIXTURE, "From: Ada Lovelace <ada@example.com>", "allow"),
    ("Write", FIXTURE, "To: grace@example.org, alan@sub.example.net", "allow"),
    ("Write", FIXTURE, "From: bob@mail.test", "allow"),
    ("Write", FIXTURE, "From: eve@thing.invalid", "allow"),
    # Hostnames are not addresses.
    ("Write", f"{REPO}/crates/postio-imap/src/z.rs",
     'const HOST: &str = "imap.mail.example.com";', "allow"),
    ("Write", f"{REPO}/crates/postio-imap/src/z.rs",
     'connect("imap.fastmail.com", 993)', "allow"),
    # Attribution trailer is allowlisted.
    ("Write", f"{REPO}/docs/x.md", "Co-Authored-By: X <noreply@anthropic.com>",
     "allow"),
    # Exempt paths carry the forbidden strings on purpose.
    ("Write", f"{REPO}/LICENSE", "Copyright (c) 2026 Diego Lapiduz", "allow"),
    ("Write", f"{REPO}/scripts/check-no-personal-data.py",
     "# blocks person@example.com", "allow"),
    ("Write", f"{REPO}/.claude/hooks/test-guard-personal-data.py",
     "x@example.com", "allow"),
    # No content at all (e.g. a Read-shaped payload).
    ("Write", FIXTURE, "", "allow"),
]


def decide(tool: str, path: str, text: str) -> str:
    key = "content" if tool == "Write" else "new_string"
    payload = json.dumps(
        {"tool_name": tool, "tool_input": {"file_path": path, key: text}}
    )
    env = {**os.environ, "CLAUDE_PROJECT_DIR": REPO}
    r = subprocess.run(
        [sys.executable, HOOK], input=payload, capture_output=True, text=True, env=env
    )
    if r.returncode != 0:
        return f"ERROR rc={r.returncode} {r.stderr.strip()}"
    if not r.stdout.strip():
        return "allow"
    return json.loads(r.stdout)["hookSpecificOutput"]["permissionDecision"]


failures = 0
for tool, path, text, want in CASES:
    got = decide(tool, path, text)
    ok = got == want
    failures += not ok
    shown = (text[:52] + "...") if len(text) > 55 else text
    print(f"  {'ok  ' if ok else 'FAIL'} {want:5} {shown!r} -> {got}")

print(f"\n{len(CASES)} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
