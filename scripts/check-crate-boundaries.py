#!/usr/bin/env python3
"""Enforce Postio's architectural crate boundaries.

The invariants (see CLAUDE.md, "Architectural invariants"):

  * ``postio-core`` must not depend on ``gtk4``/``libadwaita``. It is the
    UI-agnostic runtime -- commands in, events out -- which is what makes a
    non-GTK frontend possible later.
  * ``postio-gtk`` must not depend on ``rusqlite``/``io-imap``. The view layer
    does no SQL and speaks no protocol.

The check asks ``cargo tree`` rather than grepping source, so it catches a
violation that arrives *transitively* through some innocent-looking
intermediate crate, and it cannot be fooled by a string in a comment.

``cargo tree -p <crate>`` and not ``cargo metadata``, because features are
resolved per package and ``cargo metadata`` reports the *workspace union*.
Since ``postio-app`` turns on ``postio-core/runtime``, the union has
``postio-core`` pulling in ``postio-storage`` — and every crate depending on
``postio-core`` then looks like it depends on ``rusqlite``, including the one
crate that must not. That is an artefact of the union and not of any build:
``cargo build -p postio-gtk`` does not enable that feature, and neither does
anyone consuming ``postio-gtk`` on its own. ``cargo tree -p`` answers the
question actually being asked.

Kinds considered:

  * normal and build dependencies, transitively, from the guarded crate;
  * the guarded crate's own *direct* dev-dependencies by name (a test that
    pulls rusqlite into postio-gtk violates the invariant just as much as the
    library would), but not dev-dependencies of its dependencies, which are
    never built.

Exit status: 0 clean, 1 violation found, 2 the check itself could not run.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys

# --- The invariants ---------------------------------------------------------
#
# `banned` lists crate names that must not appear anywhere in the guarded
# crate's dependency closure. The `-sys` / companion crates are listed
# alongside the bindings they belong to so the rule cannot be side-stepped by
# depending on the lower layer directly.

RULES: dict[str, dict[str, object]] = {
    "postio-core": {
        "banned": [
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
            "libadwaita",
            "libadwaita-sys",
            "gdk4",
            "gdk4-sys",
            "gsk4-sys",
        ],
        "why": (
            "postio-core is the UI-agnostic runtime (commands in, events out). "
            "Keeping GTK out of it is what makes a second frontend possible. "
            "Widgets belong in postio-gtk; glib/gio are fine, gtk4 is not."
        ),
    },
    "postio-gtk": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            "io-imap",
        ],
        "why": (
            "postio-gtk is the view layer: command down, event up. No SQL and "
            "no protocol. Storage goes through postio-storage and mail through "
            "the MailBackend trait, both behind postio-core."
        ),
    },
}


class CheckError(Exception):
    """The check could not be run (as opposed to: the check failed)."""


def load_metadata(manifest_path: str | None, offline: bool) -> dict:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise CheckError("cargo was not found on PATH")

    cmd = [cargo, "metadata", "--format-version", "1"]
    if manifest_path:
        cmd += ["--manifest-path", manifest_path]
    if offline:
        cmd.append("--offline")

    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise CheckError(
            "`{}` failed with status {}:\n{}".format(
                " ".join(cmd), proc.returncode, proc.stderr.strip()
            )
        )
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:  # pragma: no cover - cargo bug territory
        raise CheckError(f"could not parse cargo metadata output: {exc}") from exc


def cargo_tree(
    crate: str,
    banned: str,
    edges: str,
    manifest_path: str | None,
    offline: bool,
    prefix: str = "indent",
) -> str | None:
    """Why `banned` is in `crate`'s graph, or `None` when it is not.

    `cargo tree -i` inverts the tree: it prints the banned crate and everything
    that led to it, which is a better explanation than one this script could
    assemble. A package that is not in the graph at all makes cargo exit
    non-zero with "did not match any packages", which is the clean answer.
    """
    cargo = shutil.which("cargo")
    if cargo is None:
        raise CheckError("cargo was not found on PATH")

    cmd = [cargo, "tree", "-p", crate, "-e", edges, "-i", banned, "--prefix", prefix]
    if manifest_path:
        cmd += ["--manifest-path", manifest_path]
    if offline:
        cmd.append("--offline")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        if "did not match any packages" in proc.stderr:
            return None
        raise CheckError(
            "`{}` failed with status {}:\n{}".format(
                " ".join(cmd), proc.returncode, proc.stderr.strip()
            )
        )
    return proc.stdout.strip() or None


def direct_dev_dependencies(meta: dict, crate: str) -> set[str]:
    """The names `crate` lists under `[dev-dependencies]`."""
    for pkg in meta["packages"]:
        if pkg["name"] != crate:
            continue
        if pkg["id"] not in meta.get("workspace_members", []):
            continue
        return {
            dep["name"] for dep in pkg.get("dependencies", []) if dep.get("kind") == "dev"
        }
    raise CheckError(
        f"workspace member `{crate}` was not found. The boundary rules name "
        f"crates that must exist; rename the rule in {__file__} if the crate "
        f"was intentionally renamed or removed."
    )


def find_violations(
    meta: dict, crate: str, banned: set[str], manifest_path: str | None, offline: bool
) -> dict[str, str]:
    """`{banned_crate_name: why}` for everything `crate` must not reach."""
    dev = direct_dev_dependencies(meta, crate)
    violations: dict[str, str] = {}
    for name in sorted(banned):
        if name in dev:
            violations[name] = f"direct\n{crate} --(dev-dependency)--> {name}"
            continue
        why = cargo_tree(crate, name, "normal,build", manifest_path, offline)
        if why is not None:
            violations[name] = "{}\n{}".format(
                describe_distance(crate, name, manifest_path, offline), why
            )
    return violations


def describe_distance(
    crate: str, banned: str, manifest_path: str | None, offline: bool
) -> str:
    """Whether `crate` names `banned` itself, or arrives at it through others.

    Worth saying, because the two are fixed differently: a direct dependency is
    a line to delete from a manifest, and a transitive one is an argument to
    have with whoever owns the crate in between.
    """
    depths = cargo_tree(crate, banned, "normal,build", manifest_path, offline, prefix="depth")
    if depths is None:
        return "transitive"
    # `--prefix depth` writes the depth with no separator: `1postio-gtk v0.1.0`.
    for line in depths.splitlines():
        matched = re.match(r"^(\d+)(\S+)", line)
        if matched and matched.group(1) == "1" and matched.group(2) == crate:
            return "direct"
    return "transitive"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--manifest-path",
        help="path to the workspace Cargo.toml (default: discovered from cwd)",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="pass --offline to cargo metadata (used by the self-test fixtures)",
    )
    args = parser.parse_args(argv)

    try:
        meta = load_metadata(args.manifest_path, args.offline)
    except CheckError as exc:
        print(f"crate-boundary check: {exc}", file=sys.stderr)
        return 2

    failed = False
    for crate, rule in RULES.items():
        banned = set(rule["banned"])  # type: ignore[arg-type]
        try:
            violations = find_violations(
                meta, crate, banned, args.manifest_path, args.offline
            )
        except CheckError as exc:
            print(f"crate-boundary check: {exc}", file=sys.stderr)
            return 2

        if not violations:
            print(f"ok: {crate} depends on none of: {', '.join(sorted(banned))}")
            continue

        failed = True
        for name in sorted(violations):
            print(
                f"\ncrate-boundary violation: `{crate}` must not depend on `{name}`",
                file=sys.stderr,
            )
            print(f"  offending crate:      {crate}", file=sys.stderr)
            print(f"  offending dependency: {name}", file=sys.stderr)
            print("  how:", file=sys.stderr)
            for line in violations[name].splitlines():
                print(f"    {line}", file=sys.stderr)
            print(f"  why this matters:     {rule['why']}", file=sys.stderr)

    if failed:
        print(
            "\ncrate-boundary check FAILED. See CLAUDE.md "
            '"Architectural invariants".',
            file=sys.stderr,
        )
        return 1

    print("crate-boundary check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
