#!/usr/bin/env python3
"""Enforce Postio's architectural crate boundaries.

The invariants (see CLAUDE.md, "Architectural invariants"):

  * ``postio-core`` must not depend on ``gtk4``/``libadwaita``. It is the
    UI-agnostic runtime -- commands in, events out -- which is what makes a
    non-GTK frontend possible later.
  * ``postio-gtk`` must not depend on ``rusqlite``/``turso``/``io-imap``. The view layer
    does no SQL and speaks no protocol.
  * ``postio-session`` must not depend on ``gtk4``/``libadwaita``. It is the
    composition root without a toolkit -- the store, the runtime, the engines
    and the whole verb vocabulary -- which is what makes a headless frontend
    (an MCP server; see ADR 0010) possible without giving the database a
    second writer that plays by different rules.
  * ``postio-search`` must not depend on ``rusqlite``/``turso``/``gtk4``. It is the query
    *language* -- parser, highlighter, facets -- and stays pure so the same
    query string means the same thing in the search bar, the sidebar and
    ``[filters]``; ``postio-index`` is the FTS5 executor that runs it.
  * ``postio-body`` must not depend on ``rusqlite``/``turso``/``gtk4``. It is the other
    pure leaf: the composer's document, the HTML subset, quoting and
    sanitising, kept out of ``postio-model`` only because ``ammonia`` pulls an
    HTML parser (ADR 0004) -- not because it needed a toolkit or a database.
  * ``postio-model`` must not depend on ``ammonia``/``html5ever``,
    ``rusqlite``/``turso``/``gtk4``, or ``tokio``. ADR 0004 Q1 rejected putting the
    composer's document here for exactly this reason -- dependency weight on
    the crate the whole workspace waits on -- and ADR 0007 admitted the vCard
    parser only because it brings zero dependencies of its own.
  * ``postio-config`` must not depend on ``rusqlite``/``turso``/``gtk4``. It parses and
    validates TOML and watches the file for changes; it does no SQL and links
    no toolkit.
  * ``postio-ui`` must not depend on ``gtk4``/``libadwaita``/``webkit6`` or the
    engine. It is the toolkit-free presentation logic every frontend shares
    (ADR 0019), and its own ``lib.rs`` says this check holds it to that.
  * ``postio-client`` must not depend on a toolkit, the engine, or
    ``io-imap``: it is the vocabulary between a frontend and the store's host
    (ADR 0041), and every frontend links it.
  * ``postio-tui`` must not depend on a toolkit or WebKit. It opens the store
    itself -- one app at a time has it, the terminal or the desktop app (ADR
    0041) -- so the engine and the protocol are in its graph by design; a
    toolkit is how it would stop being small (``specs/005-tui-frontend``
    FR-051).
  * ``postio-widgets`` must not depend on the store engine, the protocol, the
    host or either desktop app. It is the GTK both desktop apps draw with
    (ADR 0043), and it reaches mail only through ``postio-client``.
  * ``postio-focus`` must not depend on ``postio-gtk`` or ``postio-app``, and
    ``postio-gtk`` must not depend on ``postio-focus``: neither desktop app
    stands on the other (``specs/007-postio-focus`` FR-007).
  * ``postio-classify`` must not link anything that sends mail or reaches the
    network (``specs/007-postio-focus`` FR-132, ADR 0009), and
    ``postio-calendar`` must stay a pure leaf.
  * ``postio-ai``, the client for the person's own model, must not link a
    send path, another HTTP client, the store engine, a toolkit, or an
    inference engine: it frames HTTP to this computer with ``io-http`` and
    nothing else (``specs/007-postio-focus`` FR-165, FR-168, ADR 0009 Q1).
  * ``postio-vault``, Obsidian capture, must not link a network crate, a
    toolkit or the store engine: it appends markdown to a folder on this
    computer (``specs/007-postio-focus`` FR-180).
  * No app binary (``postio-app``, ``postio-focus``, ``postio-tui``,
    ``postio-ffi``) may link an inference engine. The local model is the
    user's own and optional (``specs/007-postio-focus`` FR-165).

Not enforced here: ADR 0001's rule that ``postio-sync`` never reaches
``io-imap``/``io-sasl``. Cargo unifies features workspace-wide, so
``postio-account``'s default ``imap`` feature is active in the resolved graph
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
    but not dev-dependencies of its dependencies, which are never built --
    unless the rule says ``"edges": "product"``, as ``postio-render``'s does:
    its invariant is about what ships, and its tests need a socket.

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

# Inference engines: runtimes that would put a language model inside a Postio
# package. The model is the user's own, run beside Postio, and optional
# (specs/007-postio-focus FR-165), so no app binary may link one. The built-in
# needs-action detector is plain code and needs none of these.
INFERENCE_ENGINES = [
    "candle-core",
    "candle-nn",
    "candle-transformers",
    "ort",
    "ort-sys",
    "tch",
    "torch-sys",
    "tract-core",
    "tract-onnx",
    "burn",
    "burn-core",
    "llama-cpp-2",
    "llama-cpp-sys-2",
    "llama_cpp",
    "llama_cpp_sys",
    "mistralrs",
    "mistralrs-core",
]

# HTTP clients, TLS and sockets: what a crate would need to reach the network.
NETWORK_CRATES = [
    "reqwest",
    "hyper",
    "h2",
    "ureq",
    "curl",
    "curl-sys",
    "isahc",
    "surf",
    "rustls",
    "tokio-rustls",
    "native-tls",
    "openssl",
    "openssl-sys",
    "socket2",
    "mio",
    "io-http",
    "pimalaya-stream",
]

RULES: dict[str, dict[str, object]] = {
    "postio-widgets": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            "turso",
            "turso_core",
            "io-imap",
            "postio-host",
            "postio-session",
            "postio-runtime",
            "postio-storage",
            "postio-sync",
            "postio-gtk",
            "postio-app",
            "postio-focus",
        ],
        "why": (
            "postio-widgets is the GTK both desktop apps draw with (ADR 0043): "
            "the message view, the composer, the small widgets and their "
            "presenters. It reaches mail only through postio-client, so it "
            "opens no store and speaks no protocol, and it depends on neither "
            "app, so neither app depends on the other through it."
        ),
    },
    "postio-focus": {
        "banned": [
            "postio-gtk",
            "postio-app",
            *INFERENCE_ENGINES,
        ],
        "why": (
            "postio-focus opens the store itself when no other Postio has it "
            "(ADR 0041), so the engine is in its graph on purpose. It draws "
            "with postio-widgets and never with the classic app's crates "
            "(specs/007-postio-focus FR-007), and it links no language model "
            "(FR-165)."
        ),
    },
    "postio-classify": {
        "banned": [
            "postio-smtp",
            "io-smtp",
            "postio-account",
            "postio-sync",
            "postio-runtime",
            "postio-transport",
            "io-imap",
            "gtk4",
            "gtk4-sys",
            "libadwaita",
            "libadwaita-sys",
            "webkit6",
            "webkit6-sys",
            *NETWORK_CRATES,
            *INFERENCE_ENGINES,
        ],
        # What ships: the classifier's tests may build a store the way every
        # store test does.
        "edges": "product",
        "why": (
            "specs/007-postio-focus FR-132 / ADR 0009: the classifier cannot "
            "send mail, by construction. Nothing that sends, and nothing that "
            "reaches the network, is in what it links, and its answer is a "
            "fixed schema with no text of its own"
        ),
    },
    "postio-calendar": {
        "banned": [
            "turso",
            "turso_core",
            "rusqlite",
            "libsqlite3-sys",
            "gtk4",
            "gtk4-sys",
            "libadwaita",
            "libadwaita-sys",
            "tokio",
            "async-std",
            *NETWORK_CRATES,
        ],
        "edges": "product",
        "why": (
            "postio-calendar is a pure leaf (specs/007-postio-focus research "
            "R9): it parses an invitation and writes a reply, and needs no "
            "store, no toolkit, no runtime and no network to do either"
        ),
    },
    "postio-ai": {
        "banned": [
            # A send path: nothing that submits, stores or syncs mail.
            "postio-smtp",
            "io-smtp",
            "postio-account",
            "postio-sync",
            "postio-runtime",
            "postio-transport",
            "io-imap",
            # The store engine, whatever it is called.
            "postio-storage",
            "turso",
            "turso_core",
            "rusqlite",
            "libsqlite3-sys",
            # A toolkit.
            "gtk4",
            "gtk4-sys",
            "libadwaita",
            "libadwaita-sys",
            "webkit6",
            "webkit6-sys",
            # Every other HTTP client. What it does keep is io-http, the
            # framing it speaks to this computer with, and what the rest of
            # the workspace turns on in it: cargo unifies features
            # workspace-wide, so io-http's TLS (pimalaya-stream, rustls) is
            # in the resolved graph however this crate asks for it -- the
            # same reason postio-sync's rule lives in its own boundary test.
            # The crate asks for io-http's `client` alone, and its own
            # manifest test holds that line; `mio` and `socket2` come with
            # the config crate's file watcher. The connection itself is
            # std's, to a loopback address or a local socket, and a
            # `ModelEndpoint` cannot name anything else.
            *[
                name
                for name in NETWORK_CRATES
                if name
                not in ("io-http", "pimalaya-stream", "rustls", "tokio-rustls", "mio", "socket2")
            ],
            *INFERENCE_ENGINES,
        ],
        # What ships: its tests run against a fake runtime and need nothing
        # more, but the invariant is about the product.
        "edges": "product",
        "why": (
            "specs/007-postio-focus FR-165, FR-168 / ADR 0009 Q1: postio-ai "
            "asks the person's own model, on this computer, questions in a "
            "fixed schema. It cannot send mail (no send path in its graph), "
            "speaks HTTP only through io-http to an endpoint that can only be "
            "this computer, "
            "opens no store and draws nothing, and carries no model of its own"
        ),
    },
    "postio-vault": {
        "banned": [
            *NETWORK_CRATES,
            "tokio",
            "gtk4",
            "gtk4-sys",
            "libadwaita",
            "libadwaita-sys",
            "webkit6",
            "webkit6-sys",
            "postio-storage",
            "turso",
            "turso_core",
            "rusqlite",
            "libsqlite3-sys",
        ],
        "edges": "product",
        "why": (
            "specs/007-postio-focus FR-180: Obsidian capture writes plain "
            "markdown into a vault on this computer, with no plugin and no "
            "network. It is file access and nothing else: no network crate, "
            "no toolkit, no store engine"
        ),
    },
    "postio-app": {
        "banned": [*INFERENCE_ENGINES],
        "why": (
            "specs/007-postio-focus FR-165: no Postio package carries a "
            "language model or an inference engine. The model is the user's "
            "own, and optional"
        ),
    },

    "postio-ui": {
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
            "rusqlite",
            "libsqlite3-sys",
            "turso",
            "turso_core",
        ],
        "why": (
            "postio-ui is the presentation logic every frontend shares -- "
            "keymap, list window, selection, palette, reader document "
            "(ADR 0019). A toolkit or the store here would put one frontend's "
            "assumptions, or a second store owner, into all of them."
        ),
    },
    "postio-storyboard": {
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
            "rusqlite",
            "libsqlite3-sys",
            "turso",
            "turso_core",
            "tokio",
        ],
        "why": (
            "postio-storyboard is the pure half of storyboards -- format, "
            "checks, comparison, review page -- that every frontend's runner "
            "calls (specs/008-storyboards). A toolkit, a store or a runtime "
            "here would make one runner's assumptions everyone's, and turn "
            "millisecond tests into window launches."
        ),
    },
    "postio-client": {
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
            "rusqlite",
            "libsqlite3-sys",
            "turso",
            "turso_core",
            "io-imap",
        ],
        "why": (
            "postio-client is what a frontend holds: commands down, events "
            "up, reads answered by the store's host (ADR 0041). It is the "
            "vocabulary every frontend links, the macOS one included, so the "
            "engine or a toolkit here would be in all of them; the store is "
            "opened by postio-host, never through this crate."
        ),
    },
    "postio-tui": {
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
            "rusqlite",
            "libsqlite3-sys",
            *INFERENCE_ENGINES,
        ],
        "why": (
            "postio-tui opens the store itself when no other Postio has it "
            "(ADR 0041), so the engine and the protocol are in its graph on "
            "purpose. It must stay small (specs/005-tui-frontend FR-051): no "
            "toolkit and no WebKit."
        ),
    },
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
            *INFERENCE_ENGINES,
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
            "io-imap",
            "postio-focus",
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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
    "postio-render": {
        "banned": [
            # toolkit / web engine
            "gtk4",
            "gtk4-sys",
            "glib",
            "gio",
            "webkit6",
            "webkit6-sys",
            # Postio crates that network or store
            "postio-transport",
            "postio-sync",
            "postio-runtime",
            "postio-storage",
            "postio-account",
            "postio-jmap",
            "postio-gmail",
            "postio-smtp",
            "io-http",
            "pimalaya-stream",
            # Blitz's own networking
            "blitz",
            "blitz-net",
            # network and TLS stacks
            "reqwest",
            "hyper",
            "h2",
            "ureq",
            "curl",
            "curl-sys",
            "isahc",
            "surf",
            "rustls",
            "tokio-rustls",
            "native-tls",
            "openssl",
            "openssl-sys",
            "socket2",
            "mio",
            "tokio",
            "async-std",
        ],
        # The product graph only: the renderer's own tests bind a loopback
        # socket to prove it never connects, and use postio-test-support,
        # which pulls in tokio. Neither ever links into the app.
        "edges": "product",
        "why": (
            "spec 006 FR-001 / ADR 0042: the renderer is incapable of a "
            "network connection by construction; remote bytes enter only "
            "through postio-runtime's RemoteImageFetcher"
        ),
    },
    "postio-config": {
        "banned": [
            "rusqlite",
            "libsqlite3-sys",
            # The engine, whatever it is currently called. `rusqlite` and
            # `libsqlite3-sys` stay listed with it: a rule keyed on a
            # dependency's *name* stops holding the moment the name changes,
            # and the whole point of this check is that the boundary does not
            # depend on anyone noticing (specs/004-turso-store T002).
            "turso",
            "turso_core",
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


def find_violations(
    meta: dict, crate: str, banned: set[str], own_dev: bool = True
) -> dict[str, list[tuple[str, str]]]:
    """Breadth-first search of `crate`'s dependency closure.

    ``own_dev`` walks the guarded crate's own dev-dependencies too; a rule
    with ``"edges": "product"`` turns it off, to guard only what ships.

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
            if current == root and own_dev:
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
            violations = find_violations(
                meta, crate, banned, own_dev=rule.get("edges") != "product"
            )
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
