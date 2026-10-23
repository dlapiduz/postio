#!/usr/bin/env python3
"""Prove `check-ci-image-packages.py` fails when the image lacks a package.

The shape it exists for: a system library added to the bare-runner list in
`.github/actions/rust-workspace` and not to `.github/ci-image/Dockerfile`, so
every pull request is green in the image and the release fails on a runner.

A guard that has never been seen to fail is a guard nobody should trust.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-ci-image-packages.py"

ACTION = """\
runs:
  using: composite
  steps:
    - name: Name the system packages
      run: |
        packages="build-essential libgtk-4-dev
          mutter mold"
"""

DOCKERFILE = """\
FROM ubuntu
RUN apt-get update \\
 && apt-get install --no-install-recommends -y \\
      build-essential \\
      libgtk-4-dev \\
{extra}      mold \\
      git \\
 && rm -rf /var/lib/apt/lists/*
"""


def sandbox(tmp: Path, extra: str) -> Path:
    root = tmp / "tree"
    (root / ".github" / "actions" / "rust-workspace").mkdir(parents=True)
    (root / ".github" / "ci-image").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)
    (root / ".github" / "actions" / "rust-workspace" / "action.yml").write_text(ACTION)
    (root / ".github" / "ci-image" / "Dockerfile").write_text(DOCKERFILE.format(extra=extra))
    shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
    return root


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return patience.run(
        [sys.executable, str(root / "scripts" / "checks" / CHECK.name)],
        capture_output=True,
        text=True,
        timeout=120,
    )


def case_a_package_missing_from_the_image_fails() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        result = run(sandbox(Path(tmp), ""))
        if result.returncode != 1 or "mutter" not in result.stderr:
            print(
                f"  FAIL: an image without mutter was accepted, or not named "
                f"(exit {result.returncode}): {result.stderr!r}",
                file=sys.stderr,
            )
            return False
    return True


def case_an_image_with_more_than_the_list_passes() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        result = run(sandbox(Path(tmp), "      mutter \\\n"))
        if result.returncode != 0:
            print(f"  FAIL: a covering image was rejected:\n{result.stderr}",
                  file=sys.stderr)
            return False
    return True


def case_the_repository_passes() -> bool:
    result = patience.run([sys.executable, str(CHECK)], capture_output=True,
                          text=True, timeout=120)
    if result.returncode != 0:
        print(f"  FAIL: the repository's own lists disagree:\n{result.stderr}",
              file=sys.stderr)
        return False
    return True


def main() -> int:
    cases = [
        ("a package missing from the image fails", case_a_package_missing_from_the_image_fails),
        ("an image holding more than the list passes", case_an_image_with_more_than_the_list_passes),
        ("the repository's own lists agree", case_the_repository_passes),
    ]
    failed = 0
    for name, case in cases:
        if case():
            print(f"ok    {name}")
        else:
            print(f"FAIL  {name}")
            failed += 1
    if failed:
        print(f"\n{failed} case(s) failed.", file=sys.stderr)
        return 1
    print("check-ci-image-packages self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
