#!/usr/bin/env python3
"""Prove the shared-tree guard denies what it must and allows what it must."""
import json
import os
import subprocess
import sys
from pathlib import Path

# The hook beside this file, not one at a hard-coded absolute path. A test
# that always read `~/src/postio/.claude/hooks/` reported on the *shared
# checkout's* copy no matter which tree it was run from -- so a fix made in a
# worktree looked untested, and CI, which has no such directory at all, would
# have been testing nothing. Found while fixing #87, whose whole subject is
# the guard being wrong about which directory it is talking about.
HOOK = str(Path(__file__).resolve().parent / "guard-shared-tree.py")

# A private per-issue worktree, and the shared checkout it must be told apart
# from. Defined up here because `decide` hands the project directory to the
# hook rather than hoping the shell exported it.
WORKTREE = os.path.expanduser("~/src/postio-worktrees/issue-27")
SHARED = os.environ.get("CLAUDE_PROJECT_DIR") or os.path.expanduser("~/src/postio")

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
    # `CLAUDE_PROJECT_DIR` is set for real, so set it here. Without it the
    # hook reads `project` as empty and its "this is not our repository at
    # all" branch can never be taken -- so the rule that a `cd` must not be
    # able to carry a command *out* of scrutiny was untestable, and a
    # regression in it invisible. An interactive shell happens not to export
    # this, which is exactly why the test must not depend on inheriting it.
    env = dict(os.environ, CLAUDE_PROJECT_DIR=SHARED)
    r = subprocess.run(
        [sys.executable, HOOK],
        input=payload,
        capture_output=True,
        text=True,
        env=env,
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

# A leading `cd` says where the command will really run, and the guard has to
# read it the way the shell does. `~` and `$HOME` are expanded by the shell
# before `cd` ever sees them, so a command naming the worktree that way is
# correct -- and the guard used to refuse it, because the target did not start
# with `/` and it fell back to the session's own cwd, which is the shared
# checkout. Issue #87, and the *second* time the worktree exemption has
# silently failed on path handling.
#
# The cwd passed alongside is deliberately the shared tree in every case: the
# whole point is that the `cd` wins over it.
HOME = os.path.expanduser("~")
worktree_cds = [
    f"cd {WORKTREE} && git add -A",
    "cd ~/src/postio-worktrees/issue-27 && git add -A",
    "cd ~/src/postio-worktrees/issue-27 && git rebase origin/main",
    "cd $HOME/src/postio-worktrees/issue-27 && git add -A",
    "cd ${HOME}/src/postio-worktrees/issue-27 && git stash",
    f"cd '{WORKTREE}' && cargo fmt --all",
    f'cd "{WORKTREE}" && git reset --hard',
    # Not the first thing on the line. The `cd` still decides where the
    # dangerous half runs, so the guard has to find it.
    f"set -e; cd {WORKTREE} && git add -A",
    f"export CARGO_TARGET_DIR=~/src/postio/target && cd {WORKTREE} && git add -A",
]
for cmd in worktree_cds:
    got = decide(cmd, cwd=SHARED)
    ok = got == "allow"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} -> {got}")
    scoped += 1

# The mirror image, which is what stops the fix from being a hole: a `cd`
# into the shared checkout is still the shared checkout, however it is
# spelled, and a relative `cd` is not a claim to be anywhere else.
shared_cds = [
    "cd ~/src/postio && git add -A",
    "cd $HOME/src/postio && git add -A",
    f"cd {SHARED} && git stash",
    "cd ~/src/postio && git rebase origin/main",
    # Relative: resolved against the cwd, which here is the shared tree.
    "cd crates && git add -A",
    "cd ./crates/postio-core && cargo fmt --all",
    "cd .. && git reset --hard",
    # A `cd` to somewhere that is not a worktree at all.
    "cd ~/src/postio-worktrees && git add -A",
]
for cmd in shared_cds:
    got = decide(cmd, cwd=SHARED)
    ok = got == "deny"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} -> {got}")
    scoped += 1

# ── Force-pushing, which a worktree does NOT get an exemption from ──────
#
# #130. Every other rule here exists because sessions share one tree and one
# index, and inside a worktree none of that is true -- so the worktree is
# exempt wholesale. A push is the exception: it does not touch the tree at
# all, it touches the *remote*, which every session shares no matter whose
# checkout the command was typed in. The exemption was hiding that.
#
# The split is between the two spellings, because they are not the same
# promise. `--force-with-lease` refuses if the remote holds anything the
# pusher has not seen, which is exactly the protection the blanket rule is
# reaching for; bare `--force` makes no such check and will happily discard
# it. And `issue-land.sh` rebases onto origin/main before pushing, so the
# second push of any already-pushed branch is necessarily non-fast-forward --
# refusing both spellings would make the landing flow impossible rather than
# safe.
force_pushes = [
    # (command, allowed in a worktree?)
    ("git push --force origin HEAD", False),
    ("git push -f origin HEAD", False),
    ("git push --force-with-lease origin HEAD", True),
    ("git push --force-with-lease=main origin HEAD", True),
    ("git push --mirror origin", False),
    ("git push --delete origin some-branch", False),
    ("git push origin HEAD", True),
    ("git push -u origin HEAD", True),
]
for cmd, allowed_in_worktree in force_pushes:
    want = "allow" if allowed_in_worktree else "deny"
    got = decide(cmd, cwd=WORKTREE)
    ok = got == want
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} {want:<5} {cmd!r} in a worktree -> {got}")
    scoped += 1

# In the shared checkout every rewriting spelling stays refused, including
# `--force-with-lease`: `main` is the branch other sessions commit to, and
# "nothing landed since I last fetched" is a much weaker promise there than
# it is on a branch only one session has.
for cmd in [
    "git push --force origin main",
    "git push -f origin main",
    "git push --force-with-lease origin main",
    "git push --mirror origin",
    "git push --delete origin main",
]:
    got = decide(cmd, cwd=SHARED)
    ok = got == "deny"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} in the shared tree -> {got}")
    scoped += 1

# A `cd` into a worktree carries the same split, since that is how a session
# that started in the shared checkout gets there.
for cmd, want in [
    (f"cd {WORKTREE} && git push --force origin HEAD", "deny"),
    (f"cd {WORKTREE} && git push --force-with-lease origin HEAD", "allow"),
]:
    got = decide(cmd, cwd=SHARED)
    ok = got == want
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} {want:<5} {cmd!r} -> {got}")
    scoped += 1

print(f"\n{len(DENY) + len(ALLOW) + scoped} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
