#!/usr/bin/env python3
"""Every crate answers to the same lint floor, and says so in one place.

The floor used to be set per crate with `#![forbid(unsafe_code)]` and
`#![warn(missing_docs)]` attributes, and it drifted exactly the way three
hand-maintained lists always do: `forbid(unsafe_code)` reached seven crates
of twelve, `warn(missing_docs)` nine, and the crates it missed were missing
it for no reason anybody had decided. `postio-config`, `postio-account` and
`postio-smtp` contained no `unsafe` at all and still did not forbid it.

`[workspace.lints]` is the one place. This check is what keeps it the one
place: a new crate that forgets `lints.workspace = true` inherits nothing
and nobody notices, because a missing lint produces no output at all. That
is the failure mode worth catching -- an invariant that degrades silently
is one you find out about years later.

# The rule

Every workspace member must either

  * inherit the workspace floor with `[lints] workspace = true`, or
  * be a listed exception with its own `[lints.rust]` table that sets
    `unsafe_code` to at least `deny`.

`EXCEPTIONS` is deliberately a literal in this file rather than a
convention: adding a crate to it is a diff somebody reviews, which is the
point. Two crates are on it today and both have a reason.

# Exit status

0 clean, 1 a crate is off the floor, 2 the check could not run.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

# Crates that cannot inherit the floor, and why. The value is the weakest
# `unsafe_code` setting the crate is allowed to declare.
#
# Neither of these is a crate that gets to use `unsafe` freely: `deny` still
# fails the build everywhere except a site that carries an explicit
# `#[allow(unsafe_code)]` and a reason. The difference from `forbid` is only
# that such a site is *possible*.
EXCEPTIONS: dict[str, str] = {
    # `gtk::ListBoxRow` carries its mailbox id in glib object data, and
    # `ObjectExt::data`/`set_data` are unsafe by construction -- glib cannot
    # know the type a key was stored under. Confined to `sidebar.rs`, behind
    # a documented `# Safety`. The test module also sets environment
    # variables, which Rust 2024 made unsafe.
    "postio-gtk": "deny",
    # Only the test module, which sets `XDG_STATE_HOME` to a temporary
    # directory. `std::env::set_var` is unsafe in Rust 2024. No library code
    # in this crate uses `unsafe`.
    "postio-app": "deny",
    # `tests/imap_body_memory.rs` installs a counting `GlobalAlloc` to prove a
    # body fetch does not materialise the whole message. Implementing that
    # trait is `unsafe impl` by definition. No library code in this crate uses
    # `unsafe` -- and note that a test target cannot opt out of `forbid`, which
    # is why this is an exception rather than a local allow.
    "postio-account": "deny",
    # `tests/validation_cost.rs` installs a counting `GlobalAlloc` to hold
    # config validation to a work budget without a stopwatch -- the same
    # technique and the same reason as `postio-account` above, and #917 is
    # why: the wall-clock assertion it replaces measured the machine, which
    # routinely has three sessions compiling on it. No library code in this
    # crate uses `unsafe`.
    "postio-config": "deny",
    # One FFI call, in `db.rs`: `OPENSSL_init_crypto(OPENSSL_INIT_NO_ATEXIT)`,
    # behind a `Once` and a documented `# Safety`. SQLCipher pulls libcrypto
    # in, libcrypto registers an `atexit` handler that frees its own state,
    # and a sync thread still writing when the process exits then encrypts a
    # page through freed memory. Declaring the symbol is the only way to say
    # "do not register that handler"; see the call site for the coredump this
    # comes from. No other `unsafe` in this crate.
    "postio-storage": "deny",
}

# Ordered weakest to strongest, so "at least as strong as" is an index test.
STRENGTH = ["allow", "warn", "deny", "forbid"]


def level_of(value: object) -> str | None:
    """The lint level out of a Cargo lint value, which may be a bare string
    or a table with a `level` key."""
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        level = value.get("level")
        return level if isinstance(level, str) else None
    return None


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    try:
        manifest = tomllib.loads((root / "Cargo.toml").read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        print(f"cannot read the workspace manifest: {exc}", file=sys.stderr)
        return 2

    workspace = manifest.get("workspace", {})
    members = workspace.get("members", [])
    floor = workspace.get("lints", {}).get("rust", {})

    floor_unsafe = level_of(floor.get("unsafe_code"))
    if floor_unsafe != "forbid":
        print(
            "[workspace.lints.rust] must set `unsafe_code = \"forbid\"`; "
            f"found {floor_unsafe!r}",
            file=sys.stderr,
        )
        return 1

    failures: list[str] = []
    checked = 0

    for member in members:
        path = root / member / "Cargo.toml"
        try:
            crate = tomllib.loads(path.read_text())
        except (OSError, tomllib.TOMLDecodeError) as exc:
            failures.append(f"{member}: cannot read Cargo.toml: {exc}")
            continue

        name = crate.get("package", {}).get("name", member)
        lints = crate.get("lints", {})
        checked += 1

        if name in EXCEPTIONS:
            if lints.get("workspace"):
                failures.append(
                    f"{name}: listed as an exception but inherits the workspace "
                    f"floor. Remove it from EXCEPTIONS in {Path(__file__).name}."
                )
                continue
            declared = level_of(lints.get("rust", {}).get("unsafe_code"))
            weakest = EXCEPTIONS[name]
            if declared is None:
                failures.append(
                    f"{name}: is an exception, so it must declare its own "
                    f"[lints.rust] unsafe_code (at least {weakest!r})."
                )
            elif declared not in STRENGTH:
                failures.append(f"{name}: unknown lint level {declared!r}.")
            elif STRENGTH.index(declared) < STRENGTH.index(weakest):
                failures.append(
                    f"{name}: unsafe_code = {declared!r} is weaker than the "
                    f"{weakest!r} its exception allows."
                )
            continue

        if not lints.get("workspace"):
            failures.append(
                f"{name}: does not inherit the lint floor. Add\n"
                f"    [lints]\n"
                f"    workspace = true\n"
                f"  to {member}/Cargo.toml, or add the crate to EXCEPTIONS in "
                f"{Path(__file__).name} with a reason."
            )

    for line in failures:
        print(f"lint floor: {line}", file=sys.stderr)

    if failures:
        print(
            f"\n{len(failures)} crate(s) off the lint floor.",
            file=sys.stderr,
        )
        return 1

    print(
        f"lint-floor check passed ({checked} crates: "
        f"{checked - len(EXCEPTIONS)} inheriting, {len(EXCEPTIONS)} audited)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
