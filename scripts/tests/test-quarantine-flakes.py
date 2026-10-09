#!/usr/bin/env python3
"""Self-test for scripts/quarantine-flakes.py.

The script decides which table-driven GTK cases to hold out of the default
run: one that failed in two places with nothing to do with it -- two pull
requests that do not touch its crate, or a nightly on main -- within the
window. Everything it decides is a pure function of the failures it was
shown, so this proves the decisions against fixtures, without the network.

Usage: scripts/tests/test-quarantine-flakes.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "quarantine-flakes.py"
spec = importlib.util.spec_from_file_location("quarantine_flakes", SCRIPT)
assert spec and spec.loader
quarantine = importlib.util.module_from_spec(spec)
# Registered before it runs: its dataclasses look their module up by name.
sys.modules["quarantine_flakes"] = quarantine
spec.loader.exec_module(quarantine)

FAILURES: list[str] = []
NOW = datetime(2026, 10, 9, 12, tzinfo=timezone.utc)


def case(name: str, condition: bool, detail: object = "") -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


LOG = """\
2026-10-09T04:40:01.1Z         PASS [   0.025s] (   4/6707) postio-account auth::tests::a
2026-10-09T04:41:02.2Z         FAIL [   5.180s] (2353/6707) postio-gtk::focus_suite settings_wiring::the_account_verbs_have_keys_on_the_focused_row
2026-10-09T04:41:03.3Z     TIMEOUT [ 360.004s] (2361/6707) postio-widgets::widgets_suite composer_inline_image::a_pasted_image
2026-10-09T05:20:00.0Z         FAIL [   5.180s] (2353/6707) postio-gtk::focus_suite settings_wiring::the_account_verbs_have_keys_on_the_focused_row
2026-10-09T05:20:01.0Z         FAIL [   0.100s] (  12/6707) postio-sync initial::tests::not_a_table_suite
2026-10-09T05:21:00.0Z \x1b[31;1m        FAIL\x1b[0m [   1.000s] (  13/6707) \x1b[35;1mpostio-gtk::focus_suite\x1b[0m \x1b[36mbar\x1b[0m\x1b[36m::\x1b[0m\x1b[34;1mcolored\x1b[0m
"""


def occurrence(test: str, source: str, related: bool = False, days: int = 1):
    return quarantine.Occurrence(
        test=test,
        source=source,
        related=related,
        at=NOW - timedelta(days=days),
        url=f"https://example.test/{source}",
    )


def main() -> int:
    # Reading a log.
    found = quarantine.failures_in_log(LOG)
    case(
        "a FAIL and a TIMEOUT in a table suite are read, each once, colours and all",
        found
        == [
            ("postio-gtk::focus_suite", "settings_wiring::the_account_verbs_have_keys_on_the_focused_row"),
            ("postio-widgets::widgets_suite", "composer_inline_image::a_pasted_image"),
            ("postio-gtk::focus_suite", "bar::colored"),
        ],
        found,
    )

    # Deciding.
    flaky = "row_menu::a_right_click"
    two_prs = [occurrence(flaky, "pr#1792"), occurrence(flaky, "pr#1805")]
    case(
        "two unrelated pull requests make a candidate",
        [c.test for c in quarantine.candidates(two_prs, set(), NOW)] == [flaky],
    )
    case(
        "the same pull request twice is one place, not two",
        quarantine.candidates([occurrence(flaky, "pr#1805"), occurrence(flaky, "pr#1805")], set(), NOW)
        == [],
    )
    case(
        "a pull request that touches the case's crate does not count",
        quarantine.candidates(
            [occurrence(flaky, "pr#1792"), occurrence(flaky, "pr#1803", related=True)], set(), NOW
        )
        == [],
    )
    case(
        "a nightly on main counts as unrelated",
        [c.test for c in quarantine.candidates(
            [occurrence(flaky, "pr#1792"), occurrence(flaky, "main@6c32f1b9")], set(), NOW
        )] == [flaky],
    )
    case(
        "a failure older than the window does not count",
        quarantine.candidates(
            [occurrence(flaky, "pr#1792"), occurrence(flaky, "pr#1700", days=9)], set(), NOW
        )
        == [],
    )
    case(
        "a case already held out is not offered again",
        quarantine.candidates(two_prs, {flaky}, NOW) == [],
    )

    # Editing the table.
    table = """\
const IGNORED: &[&str] = &[
    // Needs a portal.
    "drag_out::a_dragged_message_survives_the_portal",
];
"""
    edited = quarantine.hold_out(table, flaky, 1793)
    case(
        "a case is appended to IGNORED with the issue that brings it back",
        f'    "{flaky}",\n];' in edited and "#1793" in edited and "drag_out::" in edited,
        edited,
    )
    case("holding out twice changes nothing", quarantine.hold_out(edited, flaky, 1793) == edited)
    empty = 'const IGNORED: &[&str] = &[]; // nothing held out\n'
    edited = quarantine.hold_out(empty, flaky, 1793)
    case(
        "an empty table becomes a list holding the case",
        f'"{flaky}",' in edited and "&[]" not in edited,
        edited,
    )
    case(
        "a file with no IGNORED table is refused, not guessed at",
        _raises(lambda: quarantine.hold_out("fn main() {}\n", flaky, 1793)),
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("quarantine-flakes self-test passed.")
    return 0


def _raises(call) -> bool:
    try:
        call()
    except ValueError:
        return True
    return False


if __name__ == "__main__":
    raise SystemExit(main())
