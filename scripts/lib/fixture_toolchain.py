"""Give a self-test's throwaway Cargo project the repository's compiler.

A fixture built in a temp dir sits outside the repository, so rustup finds no
`rust-toolchain.toml` for it and falls back to its *default* toolchain, which
is whatever the machine happens to have. `test-target-sweep.py` ran that way
and stayed green after the real sweep stopped working on the pinned compiler.
`check-fixture-toolchain.py` refuses a self-test that writes a `Cargo.toml` and
runs cargo without calling this.

Use:

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))
    import fixture_toolchain
    fixture_toolchain.write(root)

Lives in `scripts/lib`, not `scripts/tests`: CI runs every `*.py` there.
"""

from __future__ import annotations

import shutil
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PIN = REPO_ROOT / "rust-toolchain.toml"


def write(root: Path) -> Path:
    """Copy the repository's `rust-toolchain.toml` into `root`; return its path."""
    destination = Path(root) / "rust-toolchain.toml"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(PIN, destination)
    return destination
