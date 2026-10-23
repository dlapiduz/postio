#!/usr/bin/env python3
"""Self-test for scripts/lib/test_audit.py.

Runs against the fixture tree in scripts/tests/fixtures/test-audit/: a
recorded `cargo nextest list --message-format json` and the source files it
names, so nothing here runs cargo. One assertion per thing the inventory and
the smell ratchet rely on (specs/012-test-suite-value, research R5 and R6).

Usage: scripts/tests/test-test-audit-lib.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "lib"))

import test_audit  # noqa: E402  -- enabled by the sys.path line above

FIXTURE = HERE / "fixtures" / "test-audit"
FAILURES: list[str] = []


def check(name: str, got: object, want: object) -> None:
    if got != want:
        FAILURES.append(f"{name}: got {got!r}, want {want!r}")
    else:
        print(f"ok  {name}")


cases = test_audit.parse_nextest_list(json.loads((FIXTURE / "nextest-list.json").read_text()), FIXTURE)
by_id = {c.id: c for c in cases}
check("one case per listed test", len(cases), 20)


def case(binary: str, name: str) -> test_audit.Case:
    return by_id[f"{binary} {name}"]


def rel(c: test_audit.Case) -> str:
    path = test_audit.case_file(c, FIXTURE)
    return path if isinstance(path, str) else str(path.relative_to(FIXTURE))


# Where a case lives (R5).
check("suite case -> tests/<suite>/<mod>.rs",
      rel(case("postio-ffi::ffi_suite", "focus_pickers::opening_succeeds")),
      "crates/postio-ffi/tests/ffi_suite/focus_pickers.rs")
check("mod.rs module -> its file",
      rel(case("postio-ffi::ffi_suite", "nested::in_mod_rs")),
      "crates/postio-ffi/tests/ffi_suite/nested/mod.rs")
check("unit case at the crate root -> lib.rs",
      rel(case("postio-storage", "tests::counts_rows")),
      "crates/postio-storage/src/lib.rs")
check("unit case -> src/<path>.rs",
      rel(case("postio-storage", "change::tests::a_change_is_seen")),
      "crates/postio-storage/src/change.rs")
check("unit case in a mod.rs",
      rel(case("postio-storage", "deep::tests::dup")),
      "crates/postio-storage/src/deep/mod.rs")
check("harness-free case -> its CASES module",
      rel(case("postio-gtk::focus_suite", "pickers::s_then_2_snoozes")),
      "crates/postio-gtk/tests/focus_suite/pickers.rs")
check("a name defined twice and unresolvable is ambiguous",
      rel(case("postio-storage", "generated::dup")), "ambiguous")

# What a case cites (R5).
gtk = case("postio-gtk::focus_suite", "pickers::s_then_2_snoozes")
check("header spec + fn item -> spec/item",
      test_audit.parse_refs(test_audit.case_file(gtk, FIXTURE), "s_then_2_snoozes"),
      ["009/FR-021"])
ctl = case("postio-focus::controller_suite", "pickers::a_number_chooses")
check("an issue in the header is kept",
      test_audit.parse_refs(test_audit.case_file(ctl, FIXTURE), "a_number_chooses"),
      ["#1234", "009/FR-021"])
bare = case("postio-storage", "change::tests::a_change_is_seen")
check("an item with no spec in scope is ?/item",
      test_audit.parse_refs(test_audit.case_file(bare, FIXTURE), "a_change_is_seen"),
      ["?/FR-003"])
check("a spec named in prose counts",
      test_audit.parse_refs(FIXTURE / "crates/postio-storage/src/lib.rs", "counts_rows"),
      ["005", "011"])


def body(c: test_audit.Case) -> str:
    return test_audit.fn_body(test_audit.case_file(c, FIXTURE), c.name.rsplit("::", 1)[-1])


check("braces in strings, chars and comments do not end a body",
      "assert_eq!" in body(case("postio-ffi::ffi_suite", "nested::in_mod_rs")), True)


# What a case smells of (R6): each smell, and a near miss that is not one.
def smells(binary: str, name: str) -> list[str]:
    c = case(binary, name)
    path = test_audit.case_file(c, FIXTURE)
    return sorted(test_audit.smells(body(c), test_audit.parse_refs(path, c.fn_name)))


CORE, FFI = "postio-core::core_suite", "postio-ffi::ffi_suite"
check("a literal against a constant is constant-only", smells(CORE, "rules::the_label_is_fixed"), ["constant-only"])
check("constant arithmetic and Type::default() are constant-only",
      smells(CORE, "rules::the_widths_add_up"), ["constant-only"])
check("a computed value against a constant is not constant-only",
      smells(CORE, "rules::a_computed_value_meets_a_constant"), [])
check("only counting mock calls is mock-count-only",
      smells(CORE, "rules::dispatch_calls_the_backend_once"), ["mock-count-only"])
check("a count beside an outcome is not mock-count-only",
      smells(CORE, "rules::dispatch_moves_the_message"), [])
check("assert!(x.is_ok()) alone is is-ok-only", smells(FFI, "focus_pickers::opening_succeeds"), ["is-ok-only"])
check("assert!(x.is_some(), msg) alone is is-ok-only",
      smells(FFI, "focus_pickers::opening_yields_something"), ["is-ok-only"])
check("is_ok() beside a value assertion is not is-ok-only",
      smells(FFI, "focus_pickers::opening_returns_the_picker"), [])
check("an early return after 'skipping' is display-skip",
      smells("postio-gtk::focus_suite", "pickers::skips_without_a_display"), ["display-skip"])
check("no ref in the case or its file is untraced", smells(FFI, "nested::in_mod_rs"), ["untraced"])
check("a call inside an assertion is not constant", smells("postio-storage", "deep::tests::dup"), [])


# Which layer a case is in, and what it costs to run (FR-002, data-model Case).
def layer(binary: str, name: str) -> tuple[str, int, bool]:
    c = case(binary, name)
    return test_audit.layer(c, test_audit.case_file(c, FIXTURE), body(c))


check("postio-focus is the controller", layer("postio-focus::controller_suite", "pickers::a_number_chooses"),
      ("controller", 1, False))
check("postio-ffi is a frontend without a display", layer(FFI, "focus_pickers::opening_succeeds"),
      ("frontend", 2, False))
check("a GTK suite is a frontend with a display",
      layer("postio-gtk::focus_suite", "pickers::s_then_2_snoozes"), ("frontend", 3, True))
check("a POSTIO-MEASUREMENT file is the measurement tier", layer(CORE, "slow::measures_cost"),
      ("measurement", 0, False))
check("a case that reads the repository is meta", layer(CORE, "docs::the_reference_matches"), ("meta", 0, False))
check("a case that reads the repository through a helper is meta",
      layer(CORE, "docs::the_reference_is_current_through_a_helper"), ("meta", 0, False))
check("everything else is a rule", layer("postio-storage", "tests::counts_rows"), ("rule", 0, False))

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("test-audit-lib: all cases behaved")
