#!/usr/bin/env python3
"""Self-test for scripts/checks/check-crate-boundaries.py.

A guard that has never been seen to fail is not a guard. This builds throwaway
cargo workspaces in a temp dir -- with dummy path crates literally named `gtk4`,
`libadwaita`, `rusqlite`, `io-imap`, `ammonia` and `tokio` -- and asserts that
the boundary check passes on a clean layout and fails, naming the offending
crate *and* the offending dependency, on every way an invariant can be broken:
directly, transitively, and through a dev-dependency.

No network, and the real crate manifests are never touched.

Usage: scripts/tests/test-check-crate-boundaries.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-crate-boundaries.py"
REPO_ROOT = HERE.parent

WORKSPACE_MANIFEST = """\
[workspace]
resolver = "2"
members = ["crates/*", "vendor/*"]
"""


def write_crate(root: Path, group: str, name: str, deps: str = "", dev_deps: str = "") -> None:
    crate = root / group / name
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "src" / "lib.rs").write_text("pub fn noop() {}\n")
    manifest = (
        f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f"[dependencies]\n{deps}\n"
    )
    if dev_deps:
        manifest += f"\n[dev-dependencies]\n{dev_deps}\n"
    (crate / "Cargo.toml").write_text(manifest)


def build_fixture(
    root: Path,
    *,
    core_deps: str = "",
    gtk_deps: str = "",
    gtk_dev_deps: str = "",
    session_deps: str = "",
    session_dev_deps: str = "",
    search_deps: str = "",
    body_deps: str = "",
    model_deps: str = "",
    config_deps: str = "",
    helper_deps: str = "",
    include_gtk: bool = True,
) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    (root / "Cargo.toml").write_text(WORKSPACE_MANIFEST)
    # The crates that carry invariants.
    write_crate(root, "crates", "postio-core", core_deps)
    if include_gtk:
        write_crate(root, "crates", "postio-gtk", gtk_deps, gtk_dev_deps)
    write_crate(root, "crates", "postio-session", session_deps, session_dev_deps)
    write_crate(root, "crates", "postio-search", search_deps)
    write_crate(root, "crates", "postio-body", body_deps)
    write_crate(root, "crates", "postio-model", model_deps)
    write_crate(root, "crates", "postio-config", config_deps)
    write_crate(root, "crates", "helper", helper_deps)
    # Stand-ins for the real third-party crates, so nothing is fetched.
    for banned in ("gtk4", "libadwaita", "rusqlite", "io-imap", "ammonia", "tokio"):
        write_crate(root, "vendor", banned)
    return root / "Cargo.toml"


def run_check(manifest: Path, offline: bool = True) -> subprocess.CompletedProcess:
    cmd = [sys.executable, str(CHECK), "--manifest-path", str(manifest)]
    if offline:
        # Fixtures are path-only, so cargo never needs the registry. The real
        # workspace does, so that case runs online.
        cmd.append("--offline")
    return subprocess.run(cmd, capture_output=True, text=True)


FAILURES: list[str] = []


def expect(case: str, cond: bool, detail: str) -> None:
    if cond:
        print(f"  ok   {case}: {detail}")
    else:
        print(f"  FAIL {case}: {detail}")
        FAILURES.append(f"{case}: {detail}")


def check_case(
    case: str,
    manifest: Path,
    *,
    expected_status: int,
    must_mention: tuple[str, ...] = (),
    offline: bool = True,
) -> None:
    print(f"case: {case}")
    proc = run_check(manifest, offline=offline)
    output = proc.stdout + proc.stderr
    expect(
        case,
        proc.returncode == expected_status,
        f"exit status {proc.returncode} (expected {expected_status})",
    )
    for needle in must_mention:
        expect(case, needle in output, f"output names {needle!r}")
    if proc.returncode != expected_status:
        print("---- output ----")
        print(output)
        print("----------------")


def main() -> int:
    if not CHECK.exists():
        print(f"missing {CHECK}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="postio-boundary-") as tmp:
        tmp_path = Path(tmp)

        # 1. A clean workspace passes.
        check_case(
            "clean fixture passes",
            build_fixture(
                tmp_path / "clean",
                gtk_deps='postio-core = { path = "../postio-core" }\n',
            ),
            expected_status=0,
        )

        # 2. gtk4 added straight to postio-core's manifest -- the exact case in
        #    the acceptance criteria.
        check_case(
            "postio-core gains a direct gtk4 dependency",
            build_fixture(
                tmp_path / "core-gtk4",
                core_deps='gtk4 = { path = "../../vendor/gtk4" }\n',
                gtk_deps='postio-core = { path = "../postio-core" }\n',
            ),
            expected_status=1,
            must_mention=("postio-core", "gtk4", "direct"),
        )

        # 3. libadwaita smuggled in behind an intermediate crate.
        check_case(
            "postio-core gains a transitive libadwaita dependency",
            build_fixture(
                tmp_path / "core-transitive",
                core_deps='helper = { path = "../helper" }\n',
                helper_deps='libadwaita = { path = "../../vendor/libadwaita" }\n',
                gtk_deps='postio-core = { path = "../postio-core" }\n',
            ),
            expected_status=1,
            must_mention=("postio-core", "libadwaita", "helper", "transitive"),
        )

        # 4. SQL in the view layer.
        check_case(
            "postio-gtk gains a direct rusqlite dependency",
            build_fixture(
                tmp_path / "gtk-rusqlite",
                gtk_deps=(
                    'postio-core = { path = "../postio-core" }\n'
                    'rusqlite = { path = "../../vendor/rusqlite" }\n'
                ),
            ),
            expected_status=1,
            must_mention=("postio-gtk", "rusqlite"),
        )

        # 5. Protocol types in the view layer, via a test-only dependency.
        check_case(
            "postio-gtk gains an io-imap dev-dependency",
            build_fixture(
                tmp_path / "gtk-dev-imap",
                gtk_deps='postio-core = { path = "../postio-core" }\n',
                gtk_dev_deps='io-imap = { path = "../../vendor/io-imap" }\n',
            ),
            expected_status=1,
            must_mention=("postio-gtk", "io-imap", "dev-dependency"),
        )

        # 6. A guarded crate that vanished must be an error, not a silent pass.
        check_case(
            "a missing guarded crate errors out",
            build_fixture(tmp_path / "no-gtk-crate", include_gtk=False),
            expected_status=2,
            must_mention=("postio-gtk",),
        )

        # 7. postio-session is the composition root without a toolkit, and
        #    that is the whole reason it was split out of postio-app (#82).
        #    A verb added in a hurry that reaches for a widget is exactly how
        #    it would be lost, and it would be lost silently: everything
        #    would still compile and every test would still pass.
        check_case(
            "postio-session gains a direct gtk4 dependency",
            build_fixture(
                tmp_path / "session-gtk",
                session_deps='gtk4 = { path = "../../vendor/gtk4" }\n',
            ),
            expected_status=1,
            must_mention=("postio-session", "gtk4"),
        )

        # 8. …and transitively, which is the way it would actually happen:
        #    nobody writes `gtk4` into that manifest on purpose.
        check_case(
            "postio-session reaches libadwaita through another crate",
            build_fixture(
                tmp_path / "session-transitive",
                session_deps='helper = { path = "../helper" }\n',
                helper_deps='libadwaita = { path = "../../vendor/libadwaita" }\n',
            ),
            expected_status=1,
            must_mention=("postio-session", "libadwaita"),
        )

        # 9. A test is not an exemption. `postio-app`'s integration tests
        #    drive the session crate, and a dev-dependency on the toolkit
        #    would let a "headless" verb be exercised only through GTK.
        check_case(
            "postio-session gains a gtk4 dev-dependency",
            build_fixture(
                tmp_path / "session-dev",
                session_dev_deps='gtk4 = { path = "../../vendor/gtk4" }\n',
            ),
            expected_status=1,
            must_mention=("postio-session", "gtk4"),
        )

        # 10. postio-search is the query *language*, and stays pure so the
        #     same query string means the same thing in the search bar, the
        #     sidebar and `[filters]` -- postio-index is the FTS5 executor.
        check_case(
            "postio-search gains a direct rusqlite dependency",
            build_fixture(
                tmp_path / "search-rusqlite",
                search_deps='rusqlite = { path = "../../vendor/rusqlite" }\n',
            ),
            expected_status=1,
            must_mention=("postio-search", "rusqlite"),
        )

        # 11. postio-body is the other pure leaf ADR 0004 carved out --
        #     kept apart from postio-model only because ammonia pulls an
        #     HTML parser, not because it needed a toolkit.
        check_case(
            "postio-body gains a direct gtk4 dependency",
            build_fixture(
                tmp_path / "body-gtk4",
                body_deps='gtk4 = { path = "../../vendor/gtk4" }\n',
            ),
            expected_status=1,
            must_mention=("postio-body", "gtk4"),
        )

        # 12. postio-model is what the whole workspace waits on to compile;
        #     ADR 0004 Q1 rejected the composer's document here for exactly
        #     the dependency weight `ammonia` would add.
        check_case(
            "postio-model gains a direct ammonia dependency",
            build_fixture(
                tmp_path / "model-ammonia",
                model_deps='ammonia = { path = "../../vendor/ammonia" }\n',
            ),
            expected_status=1,
            must_mention=("postio-model", "ammonia"),
        )

        # 13. …and the same for a runtime it has no business scheduling on.
        check_case(
            "postio-model gains a direct tokio dependency",
            build_fixture(
                tmp_path / "model-tokio",
                model_deps='tokio = { path = "../../vendor/tokio" }\n',
            ),
            expected_status=1,
            must_mention=("postio-model", "tokio"),
        )

        # 14. postio-config parses and validates TOML; it does no SQL.
        check_case(
            "postio-config gains a direct rusqlite dependency",
            build_fixture(
                tmp_path / "config-rusqlite",
                config_deps='rusqlite = { path = "../../vendor/rusqlite" }\n',
            ),
            expected_status=1,
            must_mention=("postio-config", "rusqlite"),
        )

        # 15. And the real workspace is clean today.
        check_case(
            "the real workspace passes",
            REPO_ROOT / "Cargo.toml",
            expected_status=0,
            offline=False,
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} self-test assertion(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("\nall crate-boundary self-tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
