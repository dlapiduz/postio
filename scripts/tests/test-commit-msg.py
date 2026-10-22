#!/usr/bin/env python3
"""Self-test for scripts/commit-msg.py.

CLAUDE.md's commit rules were prose only, and 31 of the 300 commits on
main before this check were off them: `style(...)`, `fix:` with no scope,
summaries past 50 characters. Worse, a body that says "does not close #N"
closes #N when the squash lands. The checker refuses each of those and
lets through every shape git and the landing script write themselves.

Usage: scripts/tests/test-commit-msg.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "commit-msg.py"
FAILURES: list[str] = []


def run(*args: str, message: str | None = None) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        argv = [sys.executable, str(CHECK), *args]
        if message is not None:
            path = Path(tmp) / "COMMIT_EDITMSG"
            path.write_text(message)
            argv.append(str(path))
        return subprocess.run(argv, capture_output=True, text=True)


def expect(case: str, result: subprocess.CompletedProcess[str], ok: bool, says: str = "") -> None:
    if (result.returncode == 0) != ok:
        FAILURES.append(
            f"{case}: expected {'pass' if ok else 'refusal'}, got exit "
            f"{result.returncode}\n{result.stdout}{result.stderr}"
        )
    elif says and says not in result.stderr:
        FAILURES.append(f"{case}: refusal does not say {says!r}:\n{result.stderr}")
    else:
        print(f"ok  {case}")


def message(subject: str, *body: str) -> str:
    return subject + ("\n\n" + "\n".join(body) if body else "") + "\n"


GOOD_BODY = (
    "The list repainted twice per keypress because the cursor and the",
    "selection each emitted. One signal now carries both.",
    "",
    "Refs: #42",
)

# --- what must pass ---------------------------------------------------

expect("a crate scope", run("--branch", "issue-42-x",
       message=message("fix(gtk): repaint the list once per key", *GOOD_BODY)), True)
expect("several scopes", run("--branch", "issue-42-x",
       message=message("refactor(ui,gtk): share the cursor rule", *GOOD_BODY)), True)
expect("a top-level directory as scope", run("--branch", "issue-42-x",
       message=message("build(flatpak): pin the runtime", *GOOD_BODY)), True)
expect("a named non-crate area", run("--branch", "chore/x",
       message=message("ci(tooling): cache the registry")), True)
expect("a small fix needs no Refs", run("--branch", "fix/typo",
       message=message("docs(docs): fix a typo in the gotchas")), True)
expect("a spec branch refers to its spec", run("--branch", "feature/contacts",
       message=message("feat(gtk): list contacts", "Refs: specs/011-contacts T004")), True)
expect("exactly fifty characters", run("--branch", "chore/x",
       message=message("fix(gtk): " + "a" * 50)), True)
expect("git's revert", run("--branch", "issue-42-x",
       message=message('Revert "fix(gtk): repaint the list once per key with a very long name"',
                       "This reverts commit 0123456789abcdef.", "", "Refs: #42")), True)
expect("fixup is squashed away", run("--branch", "issue-42-x",
       message=message("fixup! fix(gtk): repaint the list once per key")), True)
expect("a merge commit", run("--branch", "issue-42-x",
       message=message("Merge branch 'main' into issue-42-x")), True)
expect("long URL, trailer and indented lines", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once per key",
    "See https://example.com/a/very/long/path/that/cannot/be/wrapped/at/all/ever/x",
    "",
    "    let indented_code = a_line_that_is_allowed_to_run_past_the_seventy_two_limit();",
    "",
    "Refs: #42",
    "Claude-Session: https://claude.ai/code/session_0123456789abcdefghijklmnopqrstuvwxyz",
    "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>",
)), True)
expect("comments and the scissors are not the message", run("--branch", "issue-42-x", message=(
    "fix(gtk): repaint the list once per key\n\nRefs: #42\n"
    "# Closes: #123 -- a template comment\n"
    "# ------------------------ >8 ------------------------\n"
    "diff --git a/x b/x\n" + "+" + "x" * 100 + "\n"
)), True)
expect("leaving an issue open, said safely", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once per key",
    "Leaves #12 open: its second criterion needs the store change.",
    "",
    "Refs: #42",
)), True)
expect("a PR title with its number", run("--title", "fix(gtk): repaint the list once per key (#1972)"), True)

# --- what must be refused ---------------------------------------------

expect("an unknown type", run("--branch", "chore/x",
       message=message("style(gtk): drop a redundant closure")), False, "style")
expect("no scope", run("--branch", "chore/x",
       message=message("fix: clear the three nightly failures")), False, "<type>(<scope>)")
expect("an unknown scope", run("--branch", "chore/x",
       message=message("fix(app): open hostile mail")), False, "app")
expect("one bad scope among good ones", run("--branch", "chore/x",
       message=message("fix(gtk,nope): open hostile mail")), False, "nope")
expect("a summary past fifty", run("--branch", "chore/x",
       message=message("fix(gtk): " + "a" * 51)), False, "51")
expect("a trailing period", run("--branch", "chore/x",
       message=message("fix(gtk): repaint the list once.")), False, "period")
expect("past tense", run("--branch", "chore/x",
       message=message("fix(gtk): fixed the list repaint")), False, "imperative")
expect("no blank line after the subject", run("--branch", "chore/x",
       message="fix(gtk): repaint the list once\nRefs: #42\n"), False, "blank line")
expect("a body line past 72", run("--branch", "chore/x", message=message(
    "fix(gtk): repaint the list once",
    "This sentence goes on and on well past the seventy-two column limit here.",
)), False, "72")
expect("a closing keyword", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once", "Fixes #42", "", "Refs: #42")), False, "closing keyword")
expect("a negated closing keyword", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once", "This does not close #12.", "", "Refs: #42")), False, "#12")
expect("a colon-form closing keyword", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once", "Closes: #42")), False, "closing keyword")
expect("a cross-repo closing keyword", run("--branch", "issue-42-x", message=message(
    "fix(gtk): repaint the list once", "Resolves example/repo#9", "", "Refs: #42")), False,
    "closing keyword")
expect("an issue branch without Refs", run("--branch", "issue-42-x",
       message=message("fix(gtk): repaint the list once")), False, "Refs: #42")
expect("a spec branch without its spec", run("--branch", "feature/contacts",
       message=message("feat(gtk): list contacts", "Refs: #42")), False, "Refs: specs/")
expect("a bad PR title", run("--title", "fix: Mac send-path stack overflow (#1848)"), False,
       "<type>(<scope>)")

if FAILURES:
    print("\n".join(FAILURES), file=sys.stderr)
    sys.exit(1)
