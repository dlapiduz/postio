#!/usr/bin/env python3
"""Keep the command registry the only table of default key bindings.

`postio-core`'s command registry declares a `default_binding` for every
command. For a long time `postio-config` declared its own table beside it, and
`keys.rs` said in its module doc that the registry read from there:

    `DEFAULT_BINDINGS` is the built-in map and `postio-core`'s command registry
    takes its default binding from here, so there is exactly one source of
    truth.

It never did. The registry carries its own literals, nothing compared the two,
and by #1227 they had drifted to **23 entries against 79** — with a third
hand-copy of 16 in a test asserting the first two agreed. Every one of the 56
commands the config table had never heard of reported *no binding at all* to
anything that asked that crate, so the macOS menu drew no accelerator for
`delete`, `send`, `flag` or `mark_unread`, whose keys worked perfectly, and the
settings pane let a rebind take `s` away from Flag without a word.

Nothing failed, because a subset agrees with its superset. That is the failure
mode this check exists for: drift between two tables is invisible until someone
reads a menu.

# The rule

No file under ``crates/*/src`` outside the registry may define a table mapping
command ids to key bindings. Anything needing the key a command answers to asks
`postio_core::config::Keymap`, which is the registry's defaults with `[keys]`
applied and knows every command.

Expectation tables in ``tests/`` are fine and deliberately not scanned: a test
that lists `("archive", "a")` is asserting *against* the registry, and it fails
when it disagrees. A table in ``src`` is one production code reads instead.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "crates/postio-core/src/registry.rs"

# `const NAME: &[(&str, &str)] = &[ ... ];`, and the `static` spelling.
TABLE = re.compile(
    r"(?:const|static)\s+(?P<name>[A-Z0-9_]+)\s*:\s*&(?:'static\s+)?\[\(&(?:'static\s+)?str,\s*&(?:'static\s+)?str\)\]\s*=\s*&\[(?P<body>.*?)\];",
    re.DOTALL,
)
PAIR = re.compile(r'\(\s*"(?P<id>[a-z0-9_]+)"\s*,\s*"(?P<key>[^"]+)"\s*\)')

# Key names a binding may use beyond a single character, lowercased. Kept short
# on purpose: this only has to recognise a *table of bindings*, not validate
# one -- `postio_config::keys::binding_problem` is what validates.
KEY_WORDS = {
    "return", "enter", "escape", "space", "tab", "backspace", "delete",
    "up", "down", "left", "right", "home", "end", "pageup", "pagedown",
}
MODIFIERS = ("mod+", "ctrl+", "cmd+", "alt+", "shift+", "super+", "meta+")

# Fewest binding-shaped pairs before a table counts as a binding table. Two
# could be a coincidence in some unrelated lookup; a real default table is
# dozens.
THRESHOLD = 3


def looks_like_a_binding(key: str) -> bool:
    """Whether `key` is plausibly a key binding rather than ordinary data."""
    key = key.strip()
    if not key:
        return False
    # A chord sequence like `g g` is two presses; judge the first.
    first = key.split(" ")[0]
    lowered = first.lower()
    return (
        len(first) == 1
        or lowered in KEY_WORDS
        or lowered.startswith(MODIFIERS)
    )


def binding_pairs(body: str) -> list[tuple[str, str]]:
    """The pairs that look like *command id* to *binding*.

    The left column is what separates a binding table from a key-name table.
    `postio-ui`'s `NAMED_KEYS` maps `"return" -> "Return"` and every pair in it
    is binding-shaped on the right; what it is not is a table of commands. So a
    pair whose id is itself a key name does not count towards the threshold.
    """
    return [
        (m.group("id"), m.group("key"))
        for m in PAIR.finditer(body)
        if looks_like_a_binding(m.group("key"))
        and m.group("id").lower() not in KEY_WORDS
    ]


def main() -> int:
    sources = sorted(ROOT.glob("crates/*/src/**/*.rs"))
    problems = []
    for path in sources:
        if path == REGISTRY:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in TABLE.finditer(text):
            pairs = binding_pairs(match.group("body"))
            if len(pairs) < THRESHOLD:
                continue
            line = text.count("\n", 0, match.start()) + 1
            shown = ", ".join(f"{i} = {k!r}" for i, k in pairs[:3])
            problems.append(
                f"{path.relative_to(ROOT)}:{line}: `{match.group('name')}` "
                f"maps {len(pairs)} command ids to bindings ({shown}, ...)"
            )

    if not problems:
        print(f"one-default-binding-table check passed ({len(sources)} files).")
        return 0

    print("one-default-binding-table check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} table(s) beside the registry.\n\n"
        "A second table of default bindings does not stay equal to the first.\n"
        "#1227 had three -- 16, 23 and 79 entries -- and nothing failed,\n"
        "because a subset agrees with its superset. What broke was every\n"
        "surface that asked the short table what key a command had: the macOS\n"
        "menu drew no accelerator for 56 commands whose keys worked, and the\n"
        "settings pane let a rebind silently take a key another command was\n"
        "already using.\n\n"
        "Declare the default in `crates/postio-core/src/registry.rs`, beside\n"
        "the command. To read the key a command answers to, ask\n"
        "`postio_core::config::Keymap` -- registry defaults with `[keys]`\n"
        "applied, for every command, with `mod` already expanded.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
