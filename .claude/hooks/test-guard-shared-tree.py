#!/usr/bin/env python3
"""Prove the shared-tree guard denies what it must and allows what it must."""
import json
import os
import subprocess
import sys
from pathlib import Path

# The hook beside this file, not a hard-coded path, so a worktree's fix is the one tested.
HOOK = str(Path(__file__).resolve().parent / "guard-shared-tree.py")

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
    "cargo fmt -p postio-gtk",
    "cargo fmt -p postio-core",
    # A rustfmt whose file list comes from an unscoped git query writes to every session's dirty
    # files.
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- '*.rs')",
    "rustfmt --edition 2024 $(git diff --name-only HEAD)",
    "rustfmt --edition 2024 $(git diff --name-only HEAD -- \"*.rs\"; git ls-files --others --exclude-standard -- \"*.rs\")",
    "git status --porcelain | awk '{print $2}' | xargs rustfmt --edition 2024",
    "git ls-files --others --exclude-standard -- '*.rs' | xargs -r rustfmt",
    "git diff --name-only HEAD | xargs rustfmt --edition 2024",
    "rustfmt --edition 2024 $(find crates -name '*.rs')",
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
    "git commit --only crates/postio-core -m 'feat: x'",
    "git commit -- crates/postio-core",
    "git commit -m 'wip' -- crates/postio-core/src/lib.rs",
    "git stash show",
    "cargo test --workspace --lib",  # a bare --workspace run is refused since #1131
    "git status",
    "git push",
    "git push origin main",
    "cargo test && git push",
    "git log --oneline",
    "git checkout -- crates/postio-core/src/lib.rs",
    "git commit -m 'docs: explain why git stash is unsafe'",
    "echo 'never run git reset --hard here' >> notes.txt",
    "cat > doc.md <<'EOF'\nNever run git stash or git add -A here.\nUse git reset --hard nowhere.\nEOF",
    "python3 - <<'PY'\nprint('git push is forbidden')\nPY",
    "cargo fmt -p postio-gtk --check",
    "cargo fmt --all --check",
    "rustfmt --edition 2024 crates/postio-core/src/lib.rs",
    "git commit -m 'feat: handle -- in the parser'",
    'git add CLAUDE.md && git commit -q -m "docs: x"\ncargo fmt --all --check',
    "git commit -m 'docs: x'\ngit add crates/postio-core\ncargo fmt --all --check",
    "cargo fmt -p postio-core --check\ngit commit -m 'x'",
    # The scoping pathspec is QUOTED and the guard blanks quoted spans, so the rule must read the
    # raw text for scope.
    "{ git diff --name-only --diff-filter=d HEAD -- 'crates/postio-core/*.rs'\n"
    "  git ls-files --others --exclude-standard  -- 'crates/postio-core/*.rs'\n"
    "} | xargs -r rustfmt --edition 2024",
    "{ git diff --name-only --diff-filter=d HEAD -- 'crates/postio-core/*.rs' 'crates/postio-gtk/*.rs'\n"
    "} | xargs -r rustfmt --edition 2024",
    "git status --porcelain -- crates/postio-core | awk '{print $NF}' | xargs -r rustfmt --edition 2024",
    'rustfmt --edition 2024 $(git diff --name-only HEAD -- "crates/postio-gtk/*.rs")',
    "rustfmt --edition 2024 crates/postio-core/src/action.rs crates/postio-core/src/config.rs",
    "rustfmt --check --edition 2024 $(git diff --name-only HEAD -- '*.rs')",
    "git diff --name-only HEAD -- '*.rs'",
    "git status --porcelain",
    "cat > doc.md <<'EOF'\nNever run rustfmt $(git diff --name-only HEAD) here.\nEOF",
]


def decide(
    cmd: str,
    cwd: str | None = None,
    session: str | None = None,
    worktrees: str | None = None,
    background: bool = False,
) -> str:
    body = {"tool_name": "Bash", "tool_input": {"command": cmd}}
    if background:
        body["tool_input"]["run_in_background"] = True
    if cwd:
        body["cwd"] = cwd
    if session:
        body["session_id"] = session
    payload = json.dumps(body)
    # Set CLAUDE_PROJECT_DIR: without it the "not our repository" branch can never be taken and the
    # rule that
    # a `cd` cannot carry a command out of scrutiny is untestable.
    env = dict(os.environ, CLAUDE_PROJECT_DIR=SHARED)
    if worktrees:
        # Claim cases build their worktrees somewhere disposable: a stray `.git` file under the real
        # root would be read by `/lanes` and `issue-claim.sh`.
        env["POSTIO_WORKTREES"] = worktrees
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

# A private worktree is exempt. Compare by path: `.../postio-worktrees/issue-27` is a string prefix
# of `.../postio`.
os.makedirs(WORKTREE, exist_ok=True)

for cmd in ["git add -A", "git stash", "cargo fmt --all", "git reset --hard"]:
    got = decide(cmd, cwd=WORKTREE)
    ok = got == "allow"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} in a worktree -> {got}")
    scoped += 1

    # In the shared checkout it is still refused, and with no cwd the guard fails closed.
    for where, label in ((SHARED, "shared tree"), (None, "no cwd")):
        got = decide(cmd, cwd=where)
        ok = got == "deny"
        failures += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} in {label} -> {got}")
        scoped += 1

# A leading `cd` says where the command runs. `~` and `$HOME` are expanded by the shell before `cd`
# sees them (#87).
# The cwd passed is always the shared tree: the `cd` must win over it.
HOME = os.path.expanduser("~")
worktree_cds = [
    f"cd {WORKTREE} && git add -A",
    "cd ~/src/postio-worktrees/issue-27 && git add -A",
    "cd ~/src/postio-worktrees/issue-27 && git rebase origin/main",
    "cd $HOME/src/postio-worktrees/issue-27 && git add -A",
    "cd ${HOME}/src/postio-worktrees/issue-27 && git stash",
    f"cd '{WORKTREE}' && cargo fmt --all",
    f"cd {WORKTREE} && cd crates/postio-core && cargo fmt -p postio-core",
    f"cd {WORKTREE}/crates/postio-core && cargo fmt -p postio-core",
    f"cd {WORKTREE} && python3 -c 'x' && cd crates && cd postio-core && cargo fmt --all",
    f'cd "{WORKTREE}" && git reset --hard',
    f"set -e; cd {WORKTREE} && git add -A",
    f"export CARGO_TARGET_DIR=~/src/postio/target && cd {WORKTREE} && git add -A",
]
for cmd in worktree_cds:
    got = decide(cmd, cwd=SHARED)
    ok = got == "allow"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} -> {got}")
    scoped += 1

# A `cd` into the shared checkout stays the shared checkout, however spelled.
shared_cds = [
    "cd ~/src/postio && git add -A",
    "cd $HOME/src/postio && git add -A",
    f"cd {SHARED} && git stash",
    "cd ~/src/postio && git rebase origin/main",
    "cd crates && git add -A",
    "cd ./crates/postio-core && cargo fmt --all",
    f"cd {WORKTREE} && cd ../../postio && cargo fmt --all",
    "cd .. && git reset --hard",
    "cd ~/src/postio-worktrees && git add -A",
]
for cmd in shared_cds:
    got = decide(cmd, cwd=SHARED)
    ok = got == "deny"
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} -> {got}")
    scoped += 1

# A worktree gets no exemption from a bare `--force` (#130): a push touches the shared remote.
# `--force-with-lease` is allowed there because `issue-land.sh` pushes a rebased branch.
force_pushes = [
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

# In the shared checkout every rewriting spelling stays refused, `--force-with-lease` included.
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

for cmd, want in [
    (f"cd {WORKTREE} && git push --force origin HEAD", "deny"),
    (f"cd {WORKTREE} && git push --force-with-lease origin HEAD", "allow"),
]:
    got = decide(cmd, cwd=SHARED)
    ok = got == want
    failures += not ok
    print(f"  {'ok  ' if ok else 'FAIL'} {want:<5} {cmd!r} -> {got}")
    scoped += 1

# One worktree, one session (#412): these cases reach a worktree without going near
# `issue-claim.sh`.

import json as _json
import tempfile
import time as _time


class FakeWorktree:
    """A directory shaped like a `git worktree`: a `.git` FILE naming a real
    git directory elsewhere, which is where the claim stamp lives.

    Shaped rather than real because the stamp's whole point is that it sits
    *outside* the working tree -- so a test against a plain directory would
    prove nothing about the thing that makes the mechanism invisible to
    `git status`.
    """

    def __init__(self, root: str, name: str, gitdirs: str) -> None:
        self.root = os.path.join(root, name)
        self.gitdir = os.path.join(gitdirs, name)
        os.makedirs(self.root, exist_ok=True)
        os.makedirs(self.gitdir, exist_ok=True)
        with open(os.path.join(self.root, ".git"), "w", encoding="utf-8") as fh:
            fh.write(f"gitdir: {self.gitdir}\n")

    @property
    def stamp(self) -> str:
        return os.path.join(self.gitdir, "postio-claim")

    def holder(self) -> str:
        try:
            with open(self.stamp, encoding="utf-8") as fh:
                return _json.load(fh)["session"]
        except (OSError, ValueError, KeyError):
            return ""

    def since(self) -> float:
        try:
            with open(self.stamp, encoding="utf-8") as fh:
                return float(_json.load(fh).get("since") or 0.0)
        except (OSError, ValueError):
            return 0.0

    def backdate(self, seconds: float) -> None:
        with open(self.stamp, encoding="utf-8") as fh:
            record = _json.load(fh)
        record["at"] = _time.time() - seconds
        with open(self.stamp, "w", encoding="utf-8") as fh:
            _json.dump(record, fh)

    def free(self) -> None:
        try:
            os.remove(self.stamp)
        except OSError:
            pass


claims = 0


def claim_case(label: str, got: str, want: str) -> None:
    global failures, claims
    ok = got == want
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} {want:<5} {label} -> {got}")


# Under `$HOME`, so the `~` and `$HOME` spellings expand as the shell would (#87).
with tempfile.TemporaryDirectory(
    dir=os.path.expanduser("~"), prefix=".postio-guard-test-"
) as scratch:
    WORKTREES = os.path.join(scratch, "postio-worktrees")
    gitdirs = os.path.join(scratch, "gitdirs")
    os.makedirs(WORKTREES)
    os.makedirs(gitdirs)

    def decide(  # noqa: F811 - the claim cases all speak to one worktrees root
        cmd: str,
        cwd: str | None = None,
        session: str | None = None,
        background: bool = False,
        _outer=decide,
    ) -> str:
        return _outer(cmd, cwd=cwd, session=session, worktrees=WORKTREES, background=background)

    mine = FakeWorktree(WORKTREES, "issue-9001", gitdirs)

    claim_case(
        "first session adopts a free worktree",
        decide("cargo test -p postio-core", cwd=mine.root, session="alpha"),
        "allow",
    )
    ok = mine.holder() == "alpha"
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} stamp names the session -> {mine.holder()!r}")

    # The stamp records since when; a refresh keeps it, and the Stop hook uses it to tell this
    # session's loose work from older work.
    adopted_at = mine.since()
    ok = abs(adopted_at - _time.time()) < 60
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} stamp says since when -> {adopted_at!r}")
    mine.backdate(2 * 60)
    decide("cargo test -p postio-core", cwd=mine.root, session="alpha")
    ok = mine.since() == adopted_at
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} a refresh keeps since -> {mine.since()!r}")

    for cmd in ["git add -A", "cargo fmt --all", "git reset --hard"]:
        claim_case(
            f"owner runs {cmd!r} in its own worktree",
            decide(cmd, cwd=mine.root, session="alpha"),
            "allow",
        )

    for cmd in ["cargo test -p postio-core", "git add -A", "ls"]:
        claim_case(
            f"a second session runs {cmd!r} there",
            decide(cmd, cwd=mine.root, session="beta"),
            "deny",
        )
    ok = mine.holder() == "alpha"
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} a refused session did not steal it -> {mine.holder()!r}")

    claim_case(
        "a second session cd's into it",
        decide(f"cd {mine.root} && git status", cwd=SHARED, session="beta"),
        "deny",
    )
    claim_case(
        "and the owner still may",
        decide(f"cd {mine.root} && git status", cwd=SHARED, session="alpha"),
        "allow",
    )

    for cmd in [
        f"git -C {mine.root} checkout .",
        f"echo x > {mine.root}/crates/postio-index/src/index.rs",
        f"rm -rf {mine.root}/target",
        f"git -C ~{mine.root[len(os.path.expanduser('~')):]} checkout .",
        f"rustfmt --edition 2024 $HOME{mine.root[len(os.path.expanduser('~')):]}/src/x.rs",
    ]:
        claim_case(
            f"a second session reaches in: {cmd!r}",
            decide(cmd, cwd=SHARED, session="beta"),
            "deny",
        )
        claim_case(
            f"the owner reaches in: {cmd!r}",
            decide(cmd, cwd=SHARED, session="alpha"),
            "allow",
        )

    # Reading a peer worktree must stay allowed (`/lanes` does it); only a reach-in that WRITES is
    # refused.
    for cmd in [
        f"cat {mine.root}/Cargo.toml",
        f"git -C {mine.root} log --oneline -5",
        f"git -C {mine.root} status --porcelain",
        f"ls {mine.root}/crates",
        f"grep -rn frobnicator {mine.root}/crates",
        f"cat $HOME{mine.root[len(os.path.expanduser('~')):]}/Cargo.toml",
    ]:
        claim_case(
            f"a second session reads it: {cmd!r}",
            decide(cmd, cwd=SHARED, session="beta"),
            "allow",
        )

    claim_case(
        "a heredoc mentioning the path",
        decide(
            f"cat > note.md <<'EOF'\nWork happens in {mine.root} now.\nEOF",
            cwd=SHARED,
            session="beta",
        ),
        "allow",
    )

    # A claim is a lease: a dead session must not strand a worktree.
    mine.backdate(46 * 60)
    claim_case(
        "a silent owner's claim expires",
        decide("cargo test -p postio-core", cwd=mine.root, session="beta"),
        "allow",
    )
    ok = mine.holder() == "beta"
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} and passes to whoever took it -> {mine.holder()!r}")
    ok = mine.since() > adopted_at
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} whose since starts when it took it -> {mine.since()!r}")

    # Not while the owner is merely waiting on a backgrounded `issue-land.sh --full` (#412).
    mine.backdate(20 * 60)
    claim_case(
        "a patient owner's claim holds",
        decide("cargo test -p postio-core", cwd=mine.root, session="alpha"),
        "deny",
    )

    # A `cd` inside a quoted string is prose, not a destination (#889). What matters is whether the
    # `cd` keyword is quoted, never its argument.
    for cmd in [
        f'gh issue comment 412 -b "my next command was scripts/issue-claim.sh 478 '
        f'&& cd {mine.root} && wc -l"',
        f"git commit -m 'reproduced with: cd {mine.root}; cargo test'",
        f'echo "run it as | cd {mine.root} | and see"',
    ]:
        claim_case(
            f"prose mentioning a cd: {cmd[:60]!r}…",
            decide(cmd, cwd=SHARED, session="gamma"),
            "allow",
        )

    # The other direction: before #889 a quoted `cd` in a commit message could grant the worktree
    # exemption.
    quoted_cd_out = os.path.join(WORKTREES, "issue-9003")
    claim_case(
        "a quoted cd does not grant the exemption either",
        decide(
            f"git commit -m 'as we did in && cd {quoted_cd_out}' && git add -A",
            cwd=SHARED,
            session="gamma",
        ),
        "deny",
    )

    # Fail open where there is nothing to arbitrate: no session id, or a directory no `git worktree
    # add` made.
    claim_case(
        "no session id",
        decide("git add -A", cwd=mine.root),
        "allow",
    )
    bare = os.path.join(WORKTREES, "issue-9002")
    os.makedirs(bare, exist_ok=True)
    claim_case(
        "a directory that is not a checkout",
        decide("git add -A", cwd=bare, session="gamma"),
        "allow",
    )

    theirs = FakeWorktree(WORKTREES, "issue-9003", gitdirs)
    claim_case(
        "a second session in its own worktree",
        decide("git add -A", cwd=theirs.root, session="delta"),
        "allow",
    )
    claim_case(
        "and the first is undisturbed",
        decide("git add -A", cwd=mine.root, session="beta"),
        "allow",
    )

# A cargo command creates or repairs the machine-wide jobserver first (#1104); other commands pay
# nothing.
# The pool lands in a directory of this test's own, never the live one.
import subprocess
import tempfile

with tempfile.TemporaryDirectory() as raw:
    pool = os.path.join(raw, "js")
    saved = {k: os.environ.get(k) for k in ("POSTIO_JOBSERVER_DIR", "POSTIO_JOBSERVER_TOKENS", "POSTIO_JOBSERVER_IDLE")}
    os.environ["POSTIO_JOBSERVER_DIR"] = pool
    os.environ["POSTIO_JOBSERVER_TOKENS"] = "2"
    os.environ["POSTIO_JOBSERVER_IDLE"] = "1"
    try:
        got = decide("ls crates", cwd=SHARED)
        ok = got == "allow" and not os.path.exists(os.path.join(pool, "fifo"))
        failures += not ok
        scoped += 1
        print(f"  {'ok  ' if ok else 'FAIL'} a command without cargo starts no jobserver -> {got}")

        got = decide("cargo build -p postio-core", cwd=SHARED)
        ok = got == "allow" and os.path.exists(os.path.join(pool, "fifo"))
        failures += not ok
        scoped += 1
        print(f"  {'ok  ' if ok else 'FAIL'} a cargo command finds the jobserver up -> {got}, fifo={os.path.exists(os.path.join(pool, 'fifo'))}")

        got = decide("cargo fmt --all", cwd=SHARED)
        ok = got == "deny"
        failures += not ok
        scoped += 1
        print(f"  {'ok  ' if ok else 'FAIL'} the side effect does not change a refusal -> {got}")
    finally:
        subprocess.run(["bash", os.path.join(os.path.dirname(HOOK), "..", "..", "scripts", "jobserver.sh"), "stop"],
                       capture_output=True)
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

# The loop's own shape (#1131) holds in a private worktree too.
LOOP_DENY = [
    "cargo test --workspace",
    "cargo test --workspace --no-fail-fast",
    "cargo test --all",
    "cargo nextest run --workspace",
    "cargo nextest run --workspace --all-features --no-fail-fast",
    "cd ~/src/postio-worktrees/issue-27 && cargo test --workspace",
    "scripts/issue-land.sh",
    "scripts/issue-land.sh -m 'feat(gtk): x'",
    "cd ~/src/postio-worktrees/issue-27 && scripts/issue-land.sh --full",
    "nohup scripts/issue-land.sh > land.log 2>&1 &",
    "git commit --no-verify -m 'x'",
    "git commit -n -m 'x'",
    "git commit -nm 'x'",
    "git commit --amend -n",
    "git -c core.hooksPath=/dev/null commit -m 'x'",
    "git config --unset core.hooksPath",
    "git config core.hooksPath /dev/null",
    "cargo build -j 8",
    "cargo build -j8 -p postio-core",
    "cargo test --jobs 4 -p postio-core",
    "cargo clippy --workspace --jobs=2",
    "cargo nextest run -p postio-gtk --build-jobs 4",
    "CARGO_BUILD_JOBS=8 cargo build",
    "RUST_LOG=debug cargo run -p postio-gtk",
    "export RUST_LOG=info",
    "env RUST_LOG=trace target/debug/postio",
    "gh issue create --title 'x' --body 'y'",
    "cd ~/src/postio-worktrees/issue-27 && gh issue create -t x -b y",
]
LOOP_ALLOW = [
    "cargo test --workspace --lib",
    "cargo test --workspace --lib -- quote",
    "cargo test --workspace --no-run",
    "cargo test --workspace --doc",
    "cargo check --workspace --all-targets",
    "cargo test -p postio-core",
    # A flag that merely begins with --all is not --all.
    "cargo nextest run -p postio-account -p postio-smtp -p postio-body --all-features --no-fail-fast",
    "cargo test -p postio-core --all-targets",
    "cargo nextest run -p postio-gtk --test focus_suite",
    "cargo nextest run -p postio-widgets --test widgets_suite",
    "POSTIO_WORKSPACE_TESTS=1 cargo test --workspace --no-fail-fast",
    "scripts/test-sanity.sh",
    "scripts/issue-land.sh --detach",
    "scripts/issue-land.sh --status",
    "scripts/issue-land.sh --gates-only --detach",
    "cd ~/src/postio-worktrees/issue-27 && scripts/issue-land.sh --detach",
    "git commit --amend --no-edit",
    "git commit --only crates/postio-core/src/lib.rs -m 'x'",
    "git commit -m 'never pass --no-verify'",
    "git config core.hooksPath .githooks",
    "git config --get core.hooksPath",
    "cargo nextest run -p postio-gtk -j 2",
    "cargo test -p postio-core -- --test-threads 1",
    "make -j4",
    "git commit -m 'never pass -j to cargo, nor RUST_LOG=debug'",
    "POSTIO_LOG=info,postio_sync=debug cargo run -p postio-gtk",
    "grep -rn RUST_LOG= crates",
    "scripts/issue-file.sh --title 'x' --body 'y'",
    "gh issue comment 12 --body 'y'",
    "gh issue view 12",
]
for where in (SHARED, WORKTREE):
    for cmd in LOOP_DENY:
        got = decide(cmd, cwd=where)
        ok = got == "deny"
        failures += not ok
        scoped += 1
        print(f"  {'ok  ' if ok else 'FAIL'} deny  {cmd!r} in {where} -> {got}")
    for cmd in LOOP_ALLOW:
        got = decide(cmd, cwd=where)
        ok = got == "allow"
        failures += not ok
        scoped += 1
        print(f"  {'ok  ' if ok else 'FAIL'} allow {cmd!r} in {where} -> {got}")
got = decide("scripts/issue-land.sh", cwd=WORKTREE, background=True)
ok = got == "allow"
failures += not ok
scoped += 1
print(f"  {'ok  ' if ok else 'FAIL'} allow a run_in_background land -> {got}")

print(f"\n{len(DENY) + len(ALLOW) + scoped + claims} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
