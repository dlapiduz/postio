#!/usr/bin/env python3
"""Self-test for scripts/risk-of-diff.sh.

The answer decides whether `issue-land.sh` asks CI for the whole nightly on a
pull request. Too eager and every landing pays for the full suite; too shy and
a diff that touches the shared controller, the seeded mail or the sync engine
merges having met only the sanity tier -- the way four GTK-only failures
reached main in one session. Both directions are exercised here.

Usage: scripts/tests/test-risk-of-diff.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "risk-of-diff.sh"

FAILURES: list[str] = []


def ask(files: list[str] | None) -> tuple[int, str]:
    text = "" if files is None else "".join(f"{name}\n" for name in files)
    result = patience.run(
        ["bash", str(SCRIPT)], input=text, capture_output=True, text=True, timeout=30
    )
    return result.returncode, result.stdout.strip()


def case(name: str, files: list[str] | None, answer: str, rule: str = "") -> None:
    """`answer` is the first line; `rule` a substring of the second (high only)."""
    code, out = ask(files)
    lines = out.splitlines()
    got = lines[0] if lines else ""
    why = lines[1] if len(lines) > 1 else ""
    if code != 0 or got != answer or rule not in why or (answer == "normal" and len(lines) != 1):
        FAILURES.append(f"{name}\n    expected {answer!r} ~ {rule!r}, got exit {code} {out!r}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    case("one ordinary crate is normal", ["crates/postio-core/src/lib.rs"], "normal")
    case("two ordinary crates are normal",
         ["crates/postio-core/src/lib.rs", "crates/postio-ui/src/lib.rs"], "normal")
    case("prose and tooling are normal", ["docs/PRODUCT.md", "scripts/check.sh"], "normal")

    case("the seeded demo mail is high", ["crates/postio-storage/src/seed.rs"], "high", "seed")
    case("a corpus message is high",
         ["crates/postio-model/tests/corpus/attachment-pdf.eml"], "high", "fixture")
    case("an .eml anywhere is high", ["crates/postio-render/tests/data/x.eml"], "high", "fixture")
    case("the shared controller is high", ["crates/postio-focus/src/lib.rs"], "high", "postio-focus")
    case("its tests are the shared controller too",
         ["crates/postio-focus/tests/a.rs"], "high", "postio-focus")
    case("a similarly named crate is not it", ["crates/postio-focus-extra/src/lib.rs"], "normal")
    case("the sync engine is high", ["crates/postio-sync/src/engine.rs"], "high", "postio-sync")
    case("the schema is high", ["crates/postio-storage/src/schema.rs"], "high", "migration")
    case("a schema migration script is high",
         ["crates/postio-storage/src/schema/migrate-content.sql"], "high", "migration")
    case("other storage code is normal", ["crates/postio-storage/src/store.rs"], "normal")

    case("three crates are high",
         ["crates/postio-core/src/a.rs", "crates/postio-ui/src/b.rs", "crates/postio-gtk/src/c.rs"],
         "high", "3 crates")
    case("many files in two crates are still normal",
         [f"crates/postio-core/src/{i}.rs" for i in range(9)] + ["crates/postio-ui/src/x.rs"],
         "normal")
    case("a crate is counted once however many files",
         ["crates/postio-core/src/a.rs", "crates/postio-core/src/b.rs", "crates/postio-ui/src/c.rs"],
         "normal")
    case("non-crate paths do not count as crates",
         ["crates/postio-core/src/a.rs", "crates/postio-ui/src/b.rs", "docs/x.md", "Cargo.lock"],
         "normal")

    # Fails safe the same direction ci-changes.sh does: no diff to read is not
    # evidence of a small one.
    case("no input at all is high", [], "high", "no diff")
    case("none is high too", None, "high", "no diff")

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("risk-of-diff self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
