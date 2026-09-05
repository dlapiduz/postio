#!/usr/bin/env python3
"""Self-test for scripts/unbuildable-crates.sh (#1152).

The script answers "what can this host not compile", and both directions
cost something. Too small a set and `issue-land.sh` runs a gate that dies on
system headers, which is what stopped a scripts-only change landing from a
Mac at all. Too large and a host that could have tested a crate silently
does not.

The interesting case is neither root: **postio-bench dev-depends on
postio-gtk**, so it needs WebKit and nothing in its own manifest says so.
That edge is why the set is derived rather than named, and it is the case a
hand-written list gets wrong the first time somebody adds one.

Hermetic: `pkg-config` and `cargo` are stubbed on PATH, so this asserts the
script's logic rather than the machine it runs on -- and passes identically
on a Linux box with every library and on a Mac with none.

Usage: scripts/tests/test-unbuildable-crates.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "unbuildable-crates.sh"
FAILURES: list[str] = []

# A workspace shaped like Postio's, reduced to what matters: two roots, one
# crate that reaches them only through a dev-dependency, and two that do not
# reach them at all.
WORKSPACE = {
    "packages": [
        {"name": "postio-gtk", "dependencies": [{"name": "postio-ui"}]},
        {"name": "postio-app", "dependencies": [{"name": "postio-gtk"}]},
        {"name": "postio-bench", "dependencies": [{"name": "postio-gtk"}]},
        {"name": "postio-ui", "dependencies": [{"name": "postio-core"}]},
        {"name": "postio-core", "dependencies": []},
    ]
}


def stubs(missing: list[str]) -> Path:
    """A PATH directory with a `pkg-config` that reports `missing` absent and
    a `cargo` that prints the fixture workspace."""
    directory = Path(tempfile.mkdtemp())

    absent = " ".join(missing)
    pkg_config = directory / "pkg-config"
    pkg_config.write_text(
        "#!/usr/bin/env bash\n"
        f'ABSENT="{absent}"\n'
        'if [ "$1" = "--exists" ]; then\n'
        '  for lib in $ABSENT; do [ "$2" = "$lib" ] && exit 1; done\n'
        "  exit 0\n"
        "fi\n"
        "exit 0\n"
    )
    cargo = directory / "cargo"
    cargo.write_text(
        "#!/usr/bin/env bash\n"
        f"cat <<'JSON'\n{json.dumps(WORKSPACE)}\nJSON\n"
    )
    for stub in (pkg_config, cargo):
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    return directory


def run(missing: list[str], *args: str) -> list[str]:
    directory = stubs(missing)
    environment = dict(os.environ, PATH=f"{directory}:{os.environ['PATH']}")
    result = subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True, text=True, timeout=60, env=environment,
    )
    if result.returncode != 0:
        FAILURES.append(f"exit {result.returncode} for {args}: {result.stderr}")
    return [line for line in result.stdout.split() if line]


def case(name: str, got: list[str], want: list[str]) -> None:
    ok = got == want
    print(f"{'ok   ' if ok else 'FAIL '} {name}")
    if not ok:
        FAILURES.append(f"{name}: want {want}, got {got}")


def main() -> int:
    # Every library present: nothing is unbuildable, and the answer is empty
    # rather than an error. This is the Linux case and the common one.
    case("a host with every library excludes nothing", run([]), [])

    # WebKit alone is enough. gtk4 and libadwaita have arm64 bottles and
    # webkitgtk does not, so this is the actual shape of a Mac.
    case(
        "a host missing webkitgtk cannot build the frontend",
        run(["webkitgtk-6.0"]),
        ["postio-app", "postio-bench", "postio-gtk"],
    )
    case(
        "a host missing all three says the same",
        run(["gtk4", "libadwaita-1", "webkitgtk-6.0"]),
        ["postio-app", "postio-bench", "postio-gtk"],
    )

    # The case a hand-written list gets wrong. `postio-bench` reaches the
    # frontend only through a dev-dependency; excluding the two obvious
    # crates and not this one drags the whole GTK stack back into a
    # `--workspace` build through it.
    got = run(["webkitgtk-6.0"])
    case(
        "a dev-dependency on the frontend makes a crate unbuildable too",
        ["postio-bench"] if "postio-bench" in got else [],
        ["postio-bench"],
    )

    # ...and the crates the frontend depends *on* stay buildable. The edge
    # only points one way: postio-ui is what postio-gtk needs, not the
    # reverse, and a set that swallowed it would stop a Mac testing almost
    # anything.
    case(
        "a crate the frontend depends on is still buildable",
        [name for name in got if name in {"postio-ui", "postio-core"}],
        [],
    )

    case(
        "--libs names what is missing",
        run(["webkitgtk-6.0"], "--libs"),
        ["webkitgtk-6.0"],
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("unbuildable-crates self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
