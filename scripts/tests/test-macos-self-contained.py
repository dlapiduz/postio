#!/usr/bin/env python3
"""Self-test for scripts/macos-self-contained.sh.

The first release dry run (#1714) built a `Postio.app` whose executable
loaded `libpostio_ffi.dylib` from the CI runner's target directory, by
absolute path: an app that launches on the machine that built it and on no
other (#1723). The script copies such libraries into the bundle, points the
executable at the copies, and refuses a bundle that still reaches outside
itself.

The proof that matters is the one a user would hit: delete the library from
where it was built, and the app still runs. A toy executable and dylib,
compiled here, stand in for Postio and its FFI library -- the loader does not
care which code is inside.

macOS only: it needs `cc`, `otool`, `install_name_tool` and `codesign`, and
the property is about the macOS loader. Elsewhere it stands down (exit 77);
CI's `macos-tooling` job is where it runs.

Usage: scripts/tests/test-macos-self-contained.py
Exit status: 0 all cases behaved, 1 otherwise, 77 not macOS.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial and the platform rule (#1249). `scripts/lib`, not beside
# this file, because CI runs every `scripts/tests/*.py` it finds.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above
import prereq  # noqa: E402

SCRIPTS = Path(__file__).resolve().parent.parent
SCRIPT = SCRIPTS / "macos-self-contained.sh"
REPO_ROOT = SCRIPTS.parent
# Under target/, which git ignores; see scripts/checks/check-test-sandboxes.py.
SANDBOXES = REPO_ROOT / "target" / "tmp"

FAILURES: list[str] = []

LIBRARY = 'const char *toy_greeting(void) { return "hello from the bundled library"; }\n'
PROGRAM = """\
#include <stdio.h>
const char *toy_greeting(void);
int main(void) { puts(toy_greeting()); return 0; }
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def sh(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    return patience.run(list(args), cwd=cwd, capture_output=True, text=True, timeout=120)


def toy_app(base: Path, *, link_library: bool = True) -> tuple[Path, Path]:
    """Toy.app whose executable loads build/libtoy.dylib by absolute path --
    the shape cargo's cdylib gives libpostio_ffi."""
    build = base / "build"
    build.mkdir()
    (build / "toy.c").write_text(LIBRARY, encoding="utf-8")
    (build / "main.c").write_text(
        PROGRAM if link_library else 'int main(void) { return 0; }\n', encoding="utf-8"
    )
    library = build / "libtoy.dylib"
    app = base / "Toy.app"
    executable = app / "Contents" / "MacOS" / "Toy"
    executable.parent.mkdir(parents=True)
    if link_library:
        r = sh("cc", "-dynamiclib", "-o", str(library), str(build / "toy.c"),
               "-install_name", str(library))
        assert r.returncode == 0, r.stderr
        r = sh("cc", "-o", str(executable), str(build / "main.c"), "-L", str(build), "-ltoy")
    else:
        r = sh("cc", "-o", str(executable), str(build / "main.c"))
    assert r.returncode == 0, r.stderr
    return app, library


def run(app: Path) -> subprocess.CompletedProcess:
    return sh("bash", str(SCRIPT), str(app))


def main() -> int:
    prereq.only_on("darwin", reason="it rewrites Mach-O load commands for the macOS loader")
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1
    SANDBOXES.mkdir(parents=True, exist_ok=True)

    # ── the shape that shipped broken ─────────────────────────────────
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        app, library = toy_app(Path(d))
        executable = app / "Contents" / "MacOS" / "Toy"
        r = run(app)
        case("bundling an app that loads a library by absolute path succeeds", r.returncode == 0, r.stdout + r.stderr)
        bundled = app / "Contents" / "Frameworks" / "libtoy.dylib"
        case("the library is copied into Contents/Frameworks", bundled.is_file(), str(list(app.rglob("*"))))
        links = sh("otool", "-L", str(executable)).stdout
        case(
            "the executable now loads it relative to itself",
            "@executable_path/../Frameworks/libtoy.dylib" in links and str(library) not in links,
            links,
        )
        ident = sh("otool", "-D", str(bundled)).stdout
        case("the copy's own install name is @rpath-relative", "@rpath/libtoy.dylib" in ident, ident)

        # The script rewrites; signing is the bundler's job, in this order.
        sh("codesign", "--force", "--sign", "-", str(bundled))
        sh("codesign", "--force", "--sign", "-", str(app))
        library.unlink()
        launched = sh(str(executable))
        case(
            "with the original library deleted, the app still runs -- the user's machine",
            launched.returncode == 0 and "hello from the bundled library" in launched.stdout,
            f"exit {launched.returncode}: {launched.stdout} {launched.stderr}",
        )

        again = run(app)
        case("running it twice changes nothing and still passes", again.returncode == 0, again.stdout + again.stderr)

    # ── nothing to bundle ─────────────────────────────────────────────
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        app, _ = toy_app(Path(d), link_library=False)
        r = run(app)
        case("an app that loads only system libraries passes untouched", r.returncode == 0, r.stdout + r.stderr)
        case("...and gains no Frameworks directory", not (app / "Contents" / "Frameworks").exists(), "")

    # ── a reference it cannot satisfy ─────────────────────────────────
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        app, library = toy_app(Path(d))
        library.unlink()
        r = run(app)
        out = r.stdout + r.stderr
        case("a library that is gone from the build machine too is a failure", r.returncode != 0, out)
        case("...naming the reference it could not satisfy", "libtoy.dylib" in out, out)

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("macos-self-contained self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
