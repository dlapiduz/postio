#!/usr/bin/env python3
"""Enforce Postio's architectural crate boundaries.

The invariants (see CLAUDE.md, "Architectural invariants"):

  * ``postio-core`` must not depend on ``gtk4``/``libadwaita``. It is the
    UI-agnostic runtime -- commands in, events out -- which is what makes a
    non-GTK frontend possible later.
  * ``postio-gtk`` must not depend on ``rusqlite``/``io-imap``. The view layer
    does no SQL and speaks no protocol.
  * ``postio-session`` must not depend on ``gtk4``/``libadwaita``. It is the
    composition root without a toolkit -- the store, the runtime, the engines
    and the whole verb vocabulary -- which is what makes a headless frontend
    (an MCP server; see ADR 0010) possible without giving the database a
    second writer that plays by different rules.
  * ``postio-search`` must not depend on ``rusqlite``/``gtk4``. It is the query
    *language* -- parser, highlighter, facets -- and stays pure so the same
    query string means the same thing in the search bar, the sidebar and
    ``[filters]``; ``postio-index`` is the FTS5 executor that runs it.
  * ``postio-body`` must not depend on ``rusqlite``/``gtk4``. It is the other
    pure leaf: the composer's document, the HTML subset, quoting and
    sanitising, kept out of ``postio-model`` only because ``ammonia`` pulls an
    HTML parser (ADR 0004) -- not because it needed a toolkit or a database.
  * ``postio-model`` must not depend on ``ammonia``/``html5ever``,
    ``rusqlite``/``gtk4``, or ``tokio``. ADR 0004 Q1 rejected putting the
    composer's document here for exactly this reason -- dependency weight on
    the crate the whole workspace waits on -- and ADR 0007 admitted the vCard
    parser only because it brings zero dependencies of its own.
  * ``postio-config`` must not depend on ``rusqlite``/``gtk4``. It parses and
    validates TOML and watches the file for changes; it does no SQL and links
    no toolkit.

Not enforced here: ADR 0001's rule that ``postio-sync`` never reaches
``io-imap``/``io-sasl``. Cargo unifies features workspace-wide, so
``postio-imap``'s default ``imap`` feature is active in the resolved graph
regardless of what ``postio-sync`` asks for -- a graph-based rule here would
fail on a manifest that is entirely correct. `crates/postio-sync/tests/boundary.rs`
holds that line instead, by reading the manifest text directly; see its own
doc comment for the full reasoning.

The check inspects ``cargo metadata``'s resolved dependency graph rather than
grepping source, so it catches a violation that arrives *transitively* through
some innocent-looking intermediate crate, and it cannot be fooled by a string in
a comment.

Kinds considered:

  * normal and build dependencies, transitively, from the guarded crate;
  * dev-dependencies of the guarded crate itself (a test that pulls rusqlite
    into postio-gtk violates the invariant just as much as the library would),
    but not dev-dependencies of its dependencies, which are never built.

Exit status: 0 clean, 1 violation found, 2 the check itself could not run.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from collections import deque

# --- The invariants ---------------------------------------------------------
#
# `banned` lists crate names that must not appear anywhere in the guarded
# crate's dependency closure. The `-sys` / companion crates are listed
# alongside the bindings they belong to so the rule cannot be side-stepped by
# depending on the lower layer directly.

RULES: dict[str, dict[str, object]] = {
    "postio-ffi": {
        "banned": [
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
            "libadwaita",
            "libadwaita-sys",
            "gdk4",
            "gdk4-sys",
            "gsk4-sys",
            "webkit6",
            "webkit6-sys",
        ],
        # `rusqlite` is deliberately *not* banned. postio-ffi sits above
        # postio-session, exactly where postio-app does, and the store is on
        # the other side of that composition root by design.
        "why": (
            "postio-ffi is the boundary the macOS app talks to (ADR 0019). "
            "It composes postio-session and speaks Command/Event, so it must "
            "never see a toolkit: a GTK type here would mean the seam had "
            "grown a second frontend's assumptions. It carries no "
            "macOS-specific code either, which is what lets `cargo test -p "
            "postio-ffi` run in the Linux gate and stop a Linux session "
            "breaking the macOS seam unnoticed."
        ),
    },
    "postio-gmail": {
        "banned": [
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
            "libadwaita",
            "libadwaita-sys",
            "rusqlite",
            "libsqlite3-sys",
        ],
        # io-imap and io-jmap are banned too, but by the crate's own
        # boundary.rs — the same feature-unification reason as postio-jmap's.
        "why": (
            "postio-gmail answers the MailBackend seam over the Gmail REST "
            "API (ADR 0018): a protocol leaf. No GTK and no SQL; the other "
            "protocol crates are held out by its own boundary.rs."
        ),
    },
    "postio-jmap": {
        "banned": [
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
            "libadwaita",
            "libadwaita-sys",
            "rusqlite",
            "libsqlite3-sys",
        ],
        # io-imap is banned too, but not here: workspace feature unification
        # puts it in the resolved graph however this crate's manifest asks
        # (the same reason postio-sync's rule lives in its own boundary.rs).
        # `crates/postio-jmap/tests/boundary.rs` guards the manifest.
        "why": (
            "postio-jmap answers the MailBackend seam in RFC 8620/8621 "
            "(ADR 0018): a protocol leaf like the adapter beside it. No GTK "
            "and no SQL; io-imap is kept out by its own boundary.rs, so the "
            "two protocol crates never see each other."
        ),
    },
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
    # The same list as postio-core's, and deliberately not shared with it: the
    # two crates are guarded for related but different reasons, and a single
    # constant would invite "fixing" one by loosening the other.
    "postio-session": {
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
            "postio-session is the composition root without a toolkit: the "
            "store, the runtime, the engines and the verb vocabulary. A "
            "headless frontend links this and not postio-app; the moment a "
            "verb reaches for a widget, the only remaining way to run mail "
            "commands is through GTK -- and ADR 0010's alternative, a second "
            "binary opening SQLite directly, gives the database two writers "
            "with different rules about ordering, undo and the queue."
        ),
    },
    "postio-search": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
        ],
        "why": (
            "postio-search is the query language -- parser, highlighter, "
            "facets -- not the index that executes it. postio-index is the "
            "FTS5 executor; postio-gtk, postio-runtime and postio-app all "
            "depend on postio-search directly, so the same query string has "
            "to mean the same thing in the search bar, the sidebar and "
            "[filters], which only holds if this crate does no SQL of its own."
        ),
    },
    "postio-body": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
        ],
        "why": (
            "postio-body is the composer's document, the HTML subset, "
            "quoting and sanitising -- the other shared leaf, kept out of "
            "postio-model only because ammonia pulls an HTML parser (ADR "
            "0004). It is not a database and not a toolkit, and either one "
            "arriving here would mean a leaf every frontend depends on now "
            "links what only one of them needs."
        ),
    },
    "postio-model": {
        "banned": [
            "ammonia",
            "html5ever",
            "markup5ever_rcdom",
            "rusqlite",
            "libsqlite3-sys",
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
            "tokio",
        ],
        "why": (
            "postio-model is what the whole workspace waits on to compile, "
            "which is the reason ADR 0004 Q1 rejected putting the composer's "
            "document here -- an HTML parser's dependency weight lands on "
            "every crate in the tree -- and the reason ADR 0007 admitted the "
            "vCard parser only because it brings zero dependencies of its "
            "own. Each of these is the class of dependency one of those ADRs "
            "argued out; letting any of them back in reopens that argument "
            "by accident instead of on purpose."
        ),
    },
    "postio-config": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            "gtk4",
            "gtk4-sys",
            "gtk4-macros",
        ],
        "why": (
            "postio-config parses and validates TOML and watches the file "
            "for live reload. It does no SQL and links no toolkit -- the "
            "schema is read by postio-core, postio-gtk and postio-app alike, "
            "and any of them depending on it should not be how SQLite or GTK "
            "quietly reach the other two."
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


def dep_kind_label(kinds: set[str | None]) -> str:
    if None in kinds:
        return "dependency"
    if "build" in kinds:
        return "build-dependency"
    if "dev" in kinds:
        return "dev-dependency"
    return "dependency"


def find_violations(meta: dict, crate: str, banned: set[str]) -> dict[str, list[tuple[str, str]]]:
    """Breadth-first search of `crate`'s dependency closure.

    Returns ``{banned_crate_name: shortest_path}`` where a path is a list of
    ``(crate_name, edge_kind)`` pairs starting at the guarded crate itself.
    """
    packages = {pkg["id"]: pkg for pkg in meta["packages"]}
    resolve = meta.get("resolve") or {}
    nodes = {node["id"]: node for node in resolve.get("nodes", [])}

    member_ids = [pid for pid in meta.get("workspace_members", []) if pid in packages]
    roots = [pid for pid in member_ids if packages[pid]["name"] == crate]
    if not roots:
        raise CheckError(
            f"workspace member `{crate}` was not found. The boundary rules name "
            f"crates that must exist; rename the rule in {__file__} if the crate "
            f"was intentionally renamed or removed."
        )

    root = roots[0]
    violations: dict[str, list[tuple[str, str]]] = {}
    seen = {root}
    queue: deque[tuple[str, list[tuple[str, str]]]] = deque(
        [(root, [(crate, "workspace member")])]
    )

    while queue:
        current, path = queue.popleft()
        node = nodes.get(current)
        if node is None:
            continue
        for dep in node.get("deps", []):
            dep_kinds = dep.get("dep_kinds") or [{"kind": None}]
            kinds = {entry.get("kind") for entry in dep_kinds}
            # dev-dependencies only count for the guarded crate itself: a
            # dependency's own dev-dependencies are never built.
            allowed: set[str | None] = {None, "build"}
            if current == root:
                allowed.add("dev")
            kinds &= allowed
            if not kinds:
                continue

            pkg_id = dep["pkg"]
            pkg = packages.get(pkg_id)
            if pkg is None:
                continue
            name = pkg["name"]
            next_path = path + [(name, dep_kind_label(kinds))]

            if name in banned:
                violations.setdefault(name, next_path)
                continue  # no need to walk inside a crate that is already banned
            if pkg_id not in seen:
                seen.add(pkg_id)
                queue.append((pkg_id, next_path))

    return violations


def format_path(path: list[tuple[str, str]]) -> str:
    out = path[0][0]
    for name, kind in path[1:]:
        out += f" --({kind})--> {name}"
    return out


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
            violations = find_violations(meta, crate, banned)
        except CheckError as exc:
            print(f"crate-boundary check: {exc}", file=sys.stderr)
            return 2

        if not violations:
            print(f"ok: {crate} depends on none of: {', '.join(sorted(banned))}")
            continue

        failed = True
        for name in sorted(violations):
            path = violations[name]
            direct = len(path) == 2
            print(
                f"\ncrate-boundary violation: `{crate}` must not depend on `{name}`",
                file=sys.stderr,
            )
            print(f"  offending crate:      {crate}", file=sys.stderr)
            print(f"  offending dependency: {name}", file=sys.stderr)
            print(
                "  how:                  {} ({})".format(
                    format_path(path),
                    "direct" if direct else "transitive",
                ),
                file=sys.stderr,
            )
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
