#!/usr/bin/env python3
"""Prove the shared-tree guard denies what it must and allows what it must."""
import json
import os
import subprocess
import sys

HOOK = "/home/user/src/postio/.claude/hooks/guard-shared-tree.py"

DENY = [
    "git reset --hard",
    "git reset HEAD~1",
    "git reset",
    "git reset --soft HEAD~1",
    "git reset --mixed origin/main",
    "cd /tmp && git reset --hard HEAD~1",
    "git stash",
    "git stash push -m wip",
    "git add -A",
    "git add .",
    "git commit -am 'wip'",
    "git commit -a -m 'wip'",
    "cargo fmt --all",
    "git push --force origin main",
    "git push -f",
    "git clean -fd",
    "git checkout .",
    "git checkout -- .",
    "git remote add origin git@github.com:x/y",
    "git rebase -i HEAD~3",
    # Learned since this guard was last live:
    "cargo fmt -p postio-gtk",
    "cargo fmt -p postio-core",
    # postio-0uv0: a rustfmt whose file list is derived from an UNSCOPED git
    # query writes to every session's dirty files. This is not a hypothetical
    # -- it is what /land told people to run, and a session ran it over 272
    # lines of someone else's loose work.
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- '*.rs')",
    "rustfmt --edition 2024 $(git diff --name-only HEAD)",
    # The exact command /land carried before 10f5a6f.
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- \"*.rs\"; git ls-files --others --exclude-standard -- \"*.rs\")",
    "git status --porcelain | awk '{print $2}' | xargs rustfmt --edition 2024",
    "git ls-files --others --exclude-standard -- '*.rs' | xargs -r rustfmt",
    "git diff --name-only HEAD | xargs rustfmt --edition 2024",
    # Same hazard, different listing tool.
    "rustfmt --edition 2024 $(find crates -name '*.rs')",
    # Scoped to `crates/` is not scoped -- that is every crate, which is the
    # bug wearing a pathspec.
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- 'crates/')",
]

ALLOW = [
    "git add crates/postio-core Cargo.lock",
    "git commit -m 'feat(core): add thing'",
    "cargo fmt --all --check",
    "git stash list",
    "git reset -- crates/postio-core/src/lib.rs",
    "git reset HEAD -- crates/postio-core",
    "git restore --staged crates/postio-core/src/lib.rs",
    # The shared index makes add+commit racy; these are the safe forms now.
    "git commit --only crates/postio-core -m 'feat: x'",
    "git commit -- crates/postio-core",
    "git commit -m 'wip' -- crates/postio-core/src/lib.rs",
    "git stash show",
    "cargo test --workspace",
    "git status",
    "git push",
    "git push origin main",
    "cargo test && git push",
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
    "git commit -m 'feat: handle -- in the parser'",
    # A later line must not be read as flags of an earlier command.
    'git add CLAUDE.md && git commit -q -m "docs: x"\ncargo fmt --all --check',
    "git commit -m 'docs: x'\ngit add crates/postio-core\ncargo fmt --all --check",
    "cargo fmt -p postio-core --check\ngit commit -m 'x'",
    # postio-0uv0: the forms /land now documents must all survive. The
    # pathspec that scopes them is QUOTED, and the guard blanks quoted spans
    # before matching -- so a rule that looked for the crate name in the
    # stripped haystack would refuse every correct command. It has to read the
    # raw text for scope.
    "{ git diff --name-only --diff-filter=d HEAD -- 'crates/postio-core/*.rs'\n"
    "  git ls-files --others --exclude-standard  -- 'crates/postio-core/*.rs'\n"
    "} | xargs -r rustfmt --edition 2024",
    # A bead that spans crates names each one; still scoped.
    "{ git diff --name-only --diff-filter=d HEAD -- 'crates/postio-core/*.rs' 'crates/postio-gtk/*.rs'\n"
    "} | xargs -r rustfmt --edition 2024",
    "git status --porcelain -- crates/postio-core | awk '{print $NF}' | xargs -r rustfmt --edition 2024",
    # Double quotes scope just as well as single.
    'rustfmt --edition 2024 $(git diff --name-only HEAD -- "crates/postio-gtk/*.rs")',
    # Naming the files by hand is the safest form and must never be refused.
    "rustfmt --edition 2024 crates/postio-core/src/action.rs crates/postio-core/src/config.rs",
    # Read-only: --check writes nothing, so an unscoped list is harmless.
    "rustfmt --check --edition 2024 $(git diff --name-only HEAD -- '*.rs')",
    # Listing without formatting is not formatting.
    "git diff --name-only HEAD -- '*.rs'",
    "git status --porcelain",
    # Documenting the forbidden form must not be running it.
    "cat > doc.md <<'EOF'\nNever run rustfmt $(git diff --name-only HEAD) here.\nEOF",
]


def decide(cmd: str, cwd: str | None = None) -> str:
    body = {"tool_name": "Bash", "tool_input": {"command": cmd}}
    if cwd:
        body["cwd"] = cwd
    payload = json.dumps(body)
    r = subprocess.run(
        [sys.executable, HOOK], input=payload, capture_output=True, text=True
    )
    if r.returncode != 0:
        return f"ERROR rc={r.returncode} {r.stderr.strip()}"
    if not r.stdout.strip():
        return "allow"
    return json.loads(r.stdout)["hookSpecificOutput"]["permissionDecision"]


failures = 0
scoped = 0
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

# A private per-issue worktree is exempt: nothing else writes there, so the
# commands that destroy a shared tree are the correct thing to run in it. The
# comparison has to be by path -- .../postio-worktrees/issue-27 is a string
# prefix match against .../postio and is emphatically not inside it.
WORKTREE = os.path.expanduser("~/src/postio-worktrees/issue-27")
SHARED = os.environ.get("CLAUDE_PROJECT_DIR") or os.path.expanduser("~/src/postio")
os.makedirs(WORKTREE, exist_ok=True)

for cmd in ["git add -A", "git stash", "cargo fmt --all", "git reset --hard"]:
    got = decide(cmd, cwd=WORKTREE)
    ok = got == "allow"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} in a worktree -> {got}")
    scoped += 1

    # The same command in the shared checkout must still be refused, and with
    # no cwd at all the guard must fail closed rather than assume a worktree.
    for where, label in ((SHARED, "shared tree"), (None, "no cwd")):
        got = decide(cmd, cwd=where)
        ok = got == "deny"
        failures += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} in {label} -> {got}")
        scoped += 1

print(f"\n{len(DENY) + len(ALLOW) + scoped} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
