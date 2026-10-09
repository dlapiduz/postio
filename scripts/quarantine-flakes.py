#!/usr/bin/env python3
"""Hold a flaky GTK case out of the default run once it has failed in two
places that have nothing to do with it.

A case in focus_suite or widgets_suite that fails on a pull request which
does not touch its crate, and again somewhere else unrelated -- another
such pull request, or a nightly on main -- is flaky, not broken by either.
Every pull request after that pays a CI cycle for it, and a red nightly makes
every one of them re-run the whole suite. This reads the last week's failed
CI jobs, finds those cases, and (with --apply) puts each in its suite's
IGNORED table with a comment naming the issue that brings it back, filing
that issue or adding the runs to the one already open.

    scripts/quarantine-flakes.py                # report only
    scripts/quarantine-flakes.py --days 14      # a longer window
    scripts/quarantine-flakes.py --apply        # edit IGNORED, file/update issues

--apply edits the working tree and leaves the commit and the landing to
you (or /steward): a held-out case is a change to review, not a side effect.

What it cannot see: a case that failed and then passed on the flake retry
inside a job that went green overall -- only failed jobs' logs are read,
because reading every green job's log costs more than it finds. Only the
table-driven suites can be held out this way; anything else is reported.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The table-driven suites, the crate whose changes make a failure "related",
# and the file whose IGNORED table holds a case out.
SUITES = {
    "postio-gtk::focus_suite": ("crates/postio-gtk/", "crates/postio-gtk/tests/focus_suite/main.rs"),
    "postio-widgets::widgets_suite": (
        "crates/postio-widgets/",
        "crates/postio-widgets/tests/widgets_suite/main.rs",
    ),
}

# Two unrelated places: one is bad luck, two is a pattern.
PLACES = 2
WINDOW = timedelta(days=7)

LINE = re.compile(
    r"\b(?:FAIL|TIMEOUT) \[\s*[\d.]+s\]\s+(?:\(\s*\d+/\d+\)\s+)?(\S+::\S+)\s+(\S+)"
)


@dataclass(frozen=True)
class Occurrence:
    """One failure of one case, in one place."""

    test: str
    # A pull request (`pr#1805`) or a run on main (`main@<sha>`).
    source: str
    # Whether the place changed the case's own crate.
    related: bool
    at: datetime
    url: str
    suite: str = ""


@dataclass
class Candidate:
    """A case to hold out, and the failures that make it one."""

    test: str
    suite: str
    occurrences: list[Occurrence] = field(default_factory=list)


ANSI = re.compile(r"\x1b\[[0-9;]*m")


def failures_in_log(log: str) -> list[tuple[str, str]]:
    """The (suite, case) pairs a job log failed, in table suites, each once."""
    seen: list[tuple[str, str]] = []
    for match in LINE.finditer(ANSI.sub("", log)):
        pair = (match.group(1), match.group(2))
        if pair[0] in SUITES and pair not in seen:
            seen.append(pair)
    return seen


def candidates(
    occurrences: list[Occurrence], ignored: set[str], now: datetime, window: timedelta = WINDOW
) -> list[Candidate]:
    """Cases that failed in at least PLACES unrelated places inside the window."""
    by_test: dict[str, Candidate] = {}
    for occurrence in occurrences:
        if occurrence.related or now - occurrence.at > window or occurrence.test in ignored:
            continue
        candidate = by_test.setdefault(occurrence.test, Candidate(occurrence.test, occurrence.suite))
        candidate.occurrences.append(occurrence)
    return sorted(
        (c for c in by_test.values() if len({o.source for o in c.occurrences}) >= PLACES),
        key=lambda c: c.test,
    )


def ignored_in(text: str) -> set[str]:
    """The names an IGNORED table already holds."""
    table = _table(text)
    return set(re.findall(r'"([^"]+)"', table.group(0))) if table else set()


def hold_out(text: str, test: str, issue: int) -> str:
    """`text` with `test` appended to its IGNORED table, naming `issue`."""
    table = _table(text)
    if table is None:
        raise ValueError("no `const IGNORED: &[&str]` table to hold the case in")
    if f'"{test}"' in table.group(0):
        return text
    entry = (
        f"    // Failed on CI in two places that do not touch this crate; held out\n"
        f"    // by scripts/quarantine-flakes.py until #{issue} finds the race.\n"
        f'    "{test}",\n'
    )
    body = table.group(0)
    if re.search(r"&\[\s*\]", body):
        replaced = re.sub(r"&\[\s*\];.*", "&[\n" + entry + "];", body, count=1)
    else:
        replaced = body[: body.rindex("];")] + entry + "];"
    return text[: table.start()] + replaced + text[table.end() :]


def _table(text: str):
    return re.search(r"const IGNORED: &\[&str\] = &\[(?:[^\]]*)\];[^\n]*", text)


# -- GitHub ------------------------------------------------------------------


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def gh_json(*args: str):
    return json.loads(gh(*args) or "null")


def changed_files(pr: int, cache: dict[int, list[str]]) -> list[str]:
    if pr not in cache:
        cache[pr] = [f["path"] for f in gh_json("pr", "view", str(pr), "--json", "files")["files"]]
    return cache[pr]


def gather(days: int) -> list[Occurrence]:
    """Every table-suite failure in the failed CI jobs of the last `days`."""
    since = (datetime.now(timezone.utc) - timedelta(days=days)).strftime("%Y-%m-%d")
    runs = gh_json(
        "api", "--paginate", "--slurp",
        f"repos/{{owner}}/{{repo}}/actions/runs?status=failure&created=>={since}&per_page=100",
    )
    occurrences: list[Occurrence] = []
    files: dict[int, list[str]] = {}
    for page in runs or []:
        for run in page.get("workflow_runs", []):
            prs = [p["number"] for p in run.get("pull_requests") or []]
            on_main = run.get("head_branch") == "main" and run.get("event") in ("push", "schedule")
            if not prs and not on_main:
                continue
            at = datetime.fromisoformat(run["created_at"].replace("Z", "+00:00"))
            jobs = gh_json("api", f"repos/{{owner}}/{{repo}}/actions/runs/{run['id']}/jobs?per_page=100")
            for job in jobs.get("jobs", []):
                if job.get("conclusion") != "failure":
                    continue
                try:
                    log = gh(
                        "api", "--allow-escape-sequences",
                        f"repos/{{owner}}/{{repo}}/actions/jobs/{job['id']}/logs",
                    )
                except subprocess.CalledProcessError:
                    # Expired, or still being written: nothing to read yet.
                    print(f"  (no log for job {job['id']}; skipped)", file=sys.stderr)
                    continue
                for suite, test in failures_in_log(log):
                    crate = SUITES[suite][0]
                    if on_main:
                        source, related = f"main@{run['head_sha'][:8]}", False
                    else:
                        pr = prs[0]
                        source = f"pr#{pr}"
                        related = any(path.startswith(crate) for path in changed_files(pr, files))
                    occurrences.append(
                        Occurrence(test, source, related, at, job["html_url"], suite)
                    )
    return occurrences


def issue_for(candidate: Candidate) -> int:
    """The open issue naming the case, with these runs added; or a new one."""
    found = gh_json(
        "issue", "list", "--state", "open", "--search", f'"{candidate.test}" in:body',
        "--json", "number",
    )
    runs = "\n".join(f"- {o.source}: {o.url}" for o in candidate.occurrences)
    if found:
        number = found[0]["number"]
        gh(
            "issue", "comment", str(number), "--body",
            f"Held out of the default run by scripts/quarantine-flakes.py: failed in "
            f"{len({o.source for o in candidate.occurrences})} unrelated places this week.\n\n{runs}",
        )
        return number
    body = (
        f"`{candidate.suite} {candidate.test}` failed on CI in places that do not touch "
        f"its crate, so it is flaky rather than broken by them:\n\n{runs}\n\n"
        "scripts/quarantine-flakes.py held it out of the default run (its suite's "
        "`IGNORED`), naming this issue. It still runs by name.\n\n"
        "**Acceptance**\n- The race is found and fixed, and the case passes reliably on CI.\n"
        "- Its `IGNORED` entry is removed.\n"
    )
    out = subprocess.run(
        [str(ROOT / "scripts/issue-file.sh"), "--anyway", "--title",
         f"Flaky on CI: {candidate.test}", "--body", body, "--label", "ready,p2"],
        check=True, capture_output=True, text=True,
    ).stdout
    return int(re.findall(r"/issues/(\d+)", out)[-1])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--days", type=int, default=WINDOW.days)
    parser.add_argument("--apply", action="store_true")
    arguments = parser.parse_args()

    ignored = set()
    for _, path in SUITES.values():
        ignored |= ignored_in((ROOT / path).read_text(encoding="utf-8"))
    found = candidates(
        gather(arguments.days), ignored, datetime.now(timezone.utc), timedelta(days=arguments.days)
    )
    if not found:
        print(f"no case failed in {PLACES} unrelated places in the last {arguments.days} days.")
        return 0
    for candidate in found:
        places = sorted({o.source for o in candidate.occurrences})
        print(f"{candidate.suite} {candidate.test}: {', '.join(places)}")
        if arguments.apply:
            issue = issue_for(candidate)
            path = ROOT / SUITES[candidate.suite][1]
            path.write_text(hold_out(path.read_text(encoding="utf-8"), candidate.test, issue), encoding="utf-8")
            print(f"  held out in {path.relative_to(ROOT)}, naming #{issue}")
    if not arguments.apply:
        print("\nreport only; --apply holds them out and files or updates their issues.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
