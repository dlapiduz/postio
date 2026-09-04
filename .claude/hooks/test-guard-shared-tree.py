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
    "cargo test --workspace --lib",  # a bare --workspace run is refused since #1131
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
    # `CLAUDE_PROJECT_DIR` is set for real, so set it here. Without it the
    # hook reads `project` as empty and its "this is not our repository at
    # all" branch can never be taken -- so the rule that a `cd` must not be
    # able to carry a command *out* of scrutiny was untestable, and a
    # regression in it invisible. An interactive shell happens not to export
    # this, which is exactly why the test must not depend on inheriting it.
    env = dict(os.environ, CLAUDE_PROJECT_DIR=SHARED)
    if worktrees:
        # The claim cases build their worktrees somewhere disposable rather
        # than in the real `~/src/postio-worktrees`: a leftover directory with
        # a `.git` file in it is something `/lanes` and `issue-claim.sh` would
        # both go and read.
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

# ── One worktree, one session ───────────────────────────────────────────────
#
# #412. The claim lock in `issue-claim.sh` is checked only inside
# `issue-claim.sh`, so a session that reached a worktree any other way -- told
# to work an issue directly, a path pasted from an earlier transcript, a
# resumed session whose worktree was released and recreated -- was subject to
# nothing. Two sessions edited one crate for four minutes and neither knew.
#
# These cases are about the *arriving*, not the claiming: every one of them
# reaches the worktree without going anywhere near the script.

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


# Under `$HOME`, so the `~`- and `$HOME`-spelled cases below are the same
# paths the shell would expand -- that expansion is a real branch in the guard
# and #87 is what an untested one costs.
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

    # An unclaimed worktree is taken by the first session that works in it,
    # whether or not it ever ran the claim script.
    claim_case(
        "first session adopts a free worktree",
        decide("cargo test -p postio-core", cwd=mine.root, session="alpha"),
        "allow",
    )
    ok = mine.holder() == "alpha"
    failures += not ok
    claims += 1
    print(f"  {'ok  ' if ok else 'FAIL'} stamp names the session -> {mine.holder()!r}")

    # And the owner keeps it, including the destructive commands a worktree is
    # exempt from in the first place.
    for cmd in ["git add -A", "cargo fmt --all", "git reset --hard"]:
        claim_case(
            f"owner runs {cmd!r} in its own worktree",
            decide(cmd, cwd=mine.root, session="alpha"),
            "allow",
        )

    # The failure #412 is about: a second session, arriving by cwd alone.
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

    # A `cd` into it is the same arrival, spelled differently -- and this is
    # the one the transcript in #412 actually shows.
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

    # Reaching in from outside never changes directory, so `cd_destination`
    # cannot see it. These are the commands that write into a worktree from
    # the shared checkout.
    for cmd in [
        f"git -C {mine.root} checkout .",
        f"echo x > {mine.root}/crates/postio-index/src/index.rs",
        f"rm -rf {mine.root}/target",
        # The shell expands these before any command sees them, so the guard
        # has to read them the same way.
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

    # Reading somebody else's worktree from your own is not the failure #412
    # is about, and refusing it would refuse `/lanes` -- the tool sessions are
    # told to use to find out who else is here. Only a reach-in that WRITES is
    # refused; being *in* the worktree is enough on its own, above.
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

    # Documenting a path is not writing to it, the same rule the rest of this
    # guard keeps.
    claim_case(
        "a heredoc mentioning the path",
        decide(
            f"cat > note.md <<'EOF'\nWork happens in {mine.root} now.\nEOF",
            cwd=SHARED,
            session="beta",
        ),
        "allow",
    )

    # A claim is a lease. A session that died holding one must not leave a
    # worktree nobody can ever have -- that is the failure that would get the
    # whole guard switched off.
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

    # ...but not while the owner is merely being patient. A session waiting on
    # a backgrounded `issue-land.sh --full` runs no commands for twenty
    # minutes, and stealing its worktree then is exactly #412 again.
    mine.backdate(20 * 60)
    claim_case(
        "a patient owner's claim holds",
        decide("cargo test -p postio-core", cwd=mine.root, session="alpha"),
        "deny",
    )

    # A `cd` inside a quoted string is prose, not a destination (#889). This
    # bit within an hour of #412 landing: a `gh issue comment` whose body
    # quoted the command that had just been refused was itself refused, twice,
    # because the body contained the words `&& cd <worktree> &&`.
    #
    # What matters is whether the `cd` *keyword* is quoted, never whether its
    # argument is -- `cd '<worktree>' && …` is a correct invocation and is
    # covered above.
    for cmd in [
        # The exact shape that was refused: a shell operator before the `cd`,
        # which is what makes it look like a command position, inside a
        # quoted argument that is only ever text.
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

    # The same weakness pointing the other way, which was latent rather than
    # harmless: before #889 a quoted `cd` could *grant* the worktree exemption
    # on the strength of a commit message, in the shared checkout where every
    # rule in `RULES` applies.
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

    # Fail open where there is nothing to arbitrate: no session id (the hook
    # run by hand, or by these very tests above), and a directory under the
    # worktrees root that no `git worktree add` ever made.
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

    # Two sessions in two different worktrees is the ordinary state and must
    # cost nothing.
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

# ── the jobserver side effect (#1104) ─────────────────────────────────────
#
# A command that runs cargo gets the machine-wide jobserver created (or
# repaired) first, because cargo reads MAKEFLAGS at startup and the fifo it
# names has to be there by then. A command that does not run cargo pays
# nothing. The pool lands in a directory of this test's own so the real one
# other sessions are drawing from right now is never touched.
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

# ── the loop's own shape (#1131) ───────────────────────────────────────────
#
# Not a shared-tree hazard, so these hold in a private worktree too. The
# transcripts show 427 workspace test runs and 852 whole-crate runs used as
# the inner loop against 6 runs of test-fast.sh, and 79 landings killed by a
# foreground tool call's cap. The docs say otherwise and are not read at the
# moment it matters; the hook is.
LOOP_DENY = [
    "cargo test --workspace",
    "cargo test --workspace --no-fail-fast",
    "cargo test --all",
    "cargo nextest run --workspace",
    "cd ~/src/postio-worktrees/issue-27 && cargo test --workspace",
    "scripts/issue-land.sh",
    "scripts/issue-land.sh -m 'feat(gtk): x'",
    "cd ~/src/postio-worktrees/issue-27 && scripts/issue-land.sh --full",
    "nohup scripts/issue-land.sh > land.log 2>&1 &",
]
LOOP_ALLOW = [
    "cargo test --workspace --lib",
    "cargo test --workspace --lib -- quote",
    "cargo test --workspace --no-run",
    "cargo test --workspace --doc",
    "cargo check --workspace --all-targets",
    "cargo test -p postio-core",
    "cargo nextest run -p postio-app --test app_suite",
    "POSTIO_WORKSPACE_TESTS=1 cargo test --workspace --no-fail-fast",
    "scripts/test-sanity.sh",
    "scripts/issue-land.sh --detach",
    "scripts/issue-land.sh --status",
    "scripts/issue-land.sh --gates-only --detach",
    "cd ~/src/postio-worktrees/issue-27 && scripts/issue-land.sh --detach",
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
# A backgrounded land is the other spelling of --detach.
got = decide("scripts/issue-land.sh", cwd=WORKTREE, background=True)
ok = got == "allow"
failures += not ok
scoped += 1
print(f"  {'ok  ' if ok else 'FAIL'} allow a run_in_background land -> {got}")

print(f"\n{len(DENY) + len(ALLOW) + scoped + claims} cases, {failures} failure(s)")
sys.exit(1 if failures else 0)
