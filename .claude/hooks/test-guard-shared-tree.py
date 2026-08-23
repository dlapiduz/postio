#!/usr/bin/env python3
"""Prove the shared-tree guard denies what it must and allows what it must."""
import json
import subprocess
import sys

HOOK = "/home/user/src/postio/.claude/hooks/guard-shared-tree.py"

DENY = [
    "git reset --hard",
    "cd /tmp && git reset --hard HEAD~1",
    "git stash",
    "git stash push -m wip",
    "git add -A",
    "git add .",
    "git commit -am 'wip'",
    "git commit -a -m 'wip'",
    "cargo fmt --all",
    "git push origin main",
    "git clean -fd",
    "git checkout .",
    "git checkout -- .",
    "git remote add origin git@github.com:x/y",
    "git rebase -i HEAD~3",
    "cargo test && git push",
    # Learned since this guard was last live:
    "git commit -- crates/postio-core",
    "git commit -m 'wip' -- crates/postio-core/src/lib.rs",
    "cargo fmt -p postio-gtk",
    "cargo fmt -p postio-core",
]

ALLOW = [
    "git add crates/postio-core Cargo.lock",
    "git commit -m 'feat(core): add thing'",
    "cargo fmt --all --check",
    "git stash list",
    "git stash show",
    "cargo test --workspace",
    "git status",
    "git log --oneline",
    "git checkout -- crates/postio-core/src/lib.rs",
    "git commit -m 'docs: explain why git stash is unsafe'",
    "echo 'never run git reset --hard here' >> notes.txt",
    # The case that caught the shell version: a heredoc documenting the rules.
    "cat > doc.md <<'EOF'\nNever run git stash or git add -A here.\nUse git reset --hard nowhere.\nEOF",
    "python3 - <<'PY'\nprint('git push is forbidden')\nPY",
    # The write forms are refused; the read-only checks are not.
    "cargo fmt -p postio-gtk --check",
    "cargo fmt --all --check",
    "rustfmt --edition 2024 crates/postio-core/src/lib.rs",
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- '*.rs')",
    "git commit -m 'feat: handle -- in the parser'",
]


def decide(cmd: str) -> str:
    payload = json.dumps({"tool_name": "Bash", "tool_input": {"command": cmd}})
    r = subprocess.run(
        [sys.executable, HOOK], input=payload, capture_output=True, text=True
    )
    if r.returncode != 0:
        return f"ERROR rc={r.returncode} {r.stderr.strip()}"
    if not r.stdout.strip():
        return "allow"
    return json.loads(r.stdout)["hookSpecificOutput"]["permissionDecision"]


failures = 0
for cmd in DENY:
    got = decide(cmd)
    ok = got == "deny"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} -> {got}")
for cmd in ALLOW:
    got = decide(cmd)
    ok = got == "allow"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} -> {got}")

print(f"\n{len(DENY) + len(ALLOW)} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
