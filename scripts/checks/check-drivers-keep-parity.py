#!/usr/bin/env python3
"""Each frontend drives every controller intent and reports what the others report (ADR 0045).

    scripts/checks/check-drivers-keep-parity.py [--update]

Every `postio_focus::Intent` variant must be named in each driver's non-test
code, and every `Input` variant one driver sends must be sent by the others. A
gap is allowed only as a line in scripts/checks/driver-parity-baseline.txt,
`<crate> <Enum>::<Variant>  # why`. `--update` rewrites it, keeping each line's why.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FOCUS = ROOT / "crates/postio-focus/src/lib.rs"
BASELINE = ROOT / "scripts/checks/driver-parity-baseline.txt"
DRIVERS = ["postio-gtk", "postio-ffi", "postio-tui"]


def variants(enum):
    text = FOCUS.read_text(encoding="utf-8")
    body = re.search(rf"^pub enum {enum} \{{\n(.*?)^\}}", text, re.S | re.M).group(1)
    return sorted(set(re.findall(r"^    ([A-Z]\w*)\b", body, re.M)))


def live(text):
    return "\n".join(l for l in text.split("#[cfg(test)]", 1)[0].splitlines() if not l.lstrip().startswith("//"))


def names(text, enum):
    """What this file calls `postio_focus::<enum>`: the full path, plus its imported name."""
    found = [rf"postio_focus::{enum}"]
    for group in re.findall(r"use postio_focus::\{([^}]*)\}", text) + re.findall(r"use postio_focus::(\w+(?: as \w+)?);", text):
        for item in group.split(","):
            parts = item.split()
            if parts and parts[0] == enum:
                found.append(parts[2] if len(parts) == 3 else enum)
    return found


def used(crate, enum):
    seen = set()
    for path in (ROOT / "crates" / crate / "src").rglob("*.rs"):
        text = live(path.read_text(encoding="utf-8"))
        for name in names(text, enum):
            seen.update(re.findall(rf"(?<![\w:]){re.escape(name)}::([A-Z]\w*)", text))
    return seen


def gaps():
    out = set()
    intents = variants("Intent")
    for crate in DRIVERS:
        out.update(f"{crate} Intent::{v}" for v in intents if v not in used(crate, "Intent"))
    inputs = set(variants("Input"))
    sent = {crate: used(crate, "Input") & inputs for crate in DRIVERS}
    anyone = set().union(*sent.values())
    for crate in DRIVERS:
        out.update(f"{crate} Input::{v}" for v in anyone - sent[crate])
    return out


HEADER = "# A gap without a why predates this check; a new one needs `# why`.\n"


def baseline():
    lines = BASELINE.read_text().splitlines() if BASELINE.exists() else []
    return {l.split("#")[0].strip(): l for l in lines if l.split("#")[0].strip()}


def main():
    now = gaps()
    known = baseline()
    if "--update" in sys.argv:
        BASELINE.write_text(HEADER + "".join(f"{known.get(g, g)}\n" for g in sorted(now)))
        return 0
    allowed = set(known)
    new = sorted(now - allowed)
    for gap in new:
        crate, what = gap.split()
        print(f"{crate} does not handle {what}")
    for gap in sorted(allowed - now):
        print(f"{gap} is handled now; delete its line from {BASELINE.relative_to(ROOT)}")
    if new:
        print("Drive it in that frontend (ADR 0045), or, if that app cannot have it, add the line to "
              f"{BASELINE.relative_to(ROOT)} with `# why`.")
    return 1 if new else 0


if __name__ == "__main__":
    sys.exit(main())
