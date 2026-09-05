#!/usr/bin/env python3
"""Self-test for scripts/checks/check-runtime-crossings.py.

A guard that has never been seen to fail is not a guard. This one exists
because of a defect that shipped -- `postio-66`, "there is no reactor
running" -- so the first case below is that defect, verbatim in shape: if
this check would not have caught the bug it was written for, it is worth
nothing.

Throwaway git repositories in a temp dir, each with one crate in it. The real
repository is never touched and nothing here reaches the network.

Usage: scripts/tests/test-check-runtime-crossings.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-runtime-crossings.py"

FAILURES: list[str] = []


def build_repo(root: Path, source: str) -> None:
    """A git repository with one crate whose `lib.rs` is `source`."""
    crate = root / "crates" / "postio-thing" / "src"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "lib.rs").write_text(source, encoding="utf-8")

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
    subprocess.run([*git, "add", "."], cwd=root, check=True)
    subprocess.run([*git, "commit", "-qm", "fixture"], cwd=root, check=True)


def run_check(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECK)],
        cwd=root,
        capture_output=True,
        text=True,
    )


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def case(name: str, source: str, *, should_fail: bool, expect_text: str = "") -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_repo(root, source)
        result = run_check(root)

    if should_fail:
        expect(
            name,
            result.returncode == 1,
            f"expected exit 1, got {result.returncode}\n{result.stdout}{result.stderr}",
        )
        if expect_text:
            expect(
                f"{name} — says what and where",
                expect_text in result.stderr,
                f"{expect_text!r} not in:\n{result.stderr}",
            )
    else:
        expect(
            name,
            result.returncode == 0,
            f"expected exit 0, got {result.returncode}\n{result.stdout}{result.stderr}",
        )


def main() -> int:
    print("check-runtime-crossings self-test")

    # ── the bug this check exists for ────────────────────────────────────
    case(
        "the 0.1.0 keyring await is caught",
        """
fn submit() {
    glib::spawn_future_local(async move {
        let key = AccountKey::new(address);
        secrets.store(&key, &password).await.unwrap();
    });
}
""",
        should_fail=True,
        expect_text="secrets.store",
    )

    # ── and its fix is not ───────────────────────────────────────────────
    case(
        "spawning it and taking the answer over a channel passes",
        """
fn submit() {
    let (sender, receiver) = async_channel::bounded(1);
    runtime.spawn(async move {
        let _ = sender.send(secrets.store(&key, &password).await).await;
    });
    glib::spawn_future_local(async move {
        let stored = receiver.recv().await;
    });
}
""",
        should_fail=False,
    )

    # ── the awaits a crossing is made of ─────────────────────────────────
    case(
        "a stream next passes",
        """
fn wire() {
    glib::spawn_future_local(async move {
        while let Some(event) = stream.next().await {
            apply(event);
        }
    });
}
""",
        should_fail=False,
    )

    # ── a runtime spawn nested inside the crossing is not the crossing ───
    #
    # This is the shape onboarding.rs actually has: the second write is
    # decided on the main context and performed on the runtime. Flagging it
    # would make the check unusable on the code it was written for.
    case(
        "an await inside a nested runtime.spawn is left alone",
        """
fn submit() {
    glib::spawn_future_local(async move {
        let answer = receiver.recv().await;
        let (sender, receiver) = async_channel::bounded(1);
        wiring.runtime.spawn(async move {
            let _ = sender.send(persist(&database, secrets, &written).await).await;
        });
        let stored = receiver.recv().await;
    });
}
""",
        should_fail=False,
    )

    # ── an owned-up-to exception ─────────────────────────────────────────
    case(
        "a marked await passes",
        """
fn request() {
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: `MessageSource::fetch` returns a future its
        // implementations guarantee is pollable on the main context.
        match future.await {
            Ok(answer) => deliver(answer),
            Err(message) => fail(message),
        }
    });
}
""",
        should_fail=False,
    )

    case(
        "a marker at the top of a long comment block still covers the await",
        """
fn request() {
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: `MessageSource::fetch` is a trait method, and the
        // trait's contract is that what it returns is pollable on the main
        // context -- `postio-app` implements it by spawning the runtime work
        // and handing back a channel receive. A `MailBackend` future must
        // never be returned from it directly.
        match future.await {
            Ok(answer) => deliver(answer),
            Err(message) => fail(message),
        }
    });
}
""",
        should_fail=False,
    )

    case(
        "a marker in an unrelated comment block above does not carry over",
        """
fn request() {
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: this covers the receive below and nothing else.
        let answer = receiver.recv().await;

        let bytes = part_bytes(&database, message).await;
    });
}
""",
        should_fail=True,
        expect_text="part_bytes",
    )

    case(
        "the marker does not cover the whole file",
        """
fn one() {
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: this one is fine.
        match future.await { _ => () }
    });
}

fn two() {
    glib::spawn_future_local(async move {
        secrets.store(&key, &password).await.unwrap();
    });
}
""",
        should_fail=True,
        expect_text="secrets.store",
    )

    # ── the tokio timer that this check found in the save-attachment path ─
    case(
        "an indirect tokio await is caught at the call that reaches it",
        """
fn save() {
    glib::spawn_future_local(async move {
        let outcome = part_bytes(&database, &blobs, engine, message, attachment).await;
        report(outcome);
    });
}
""",
        should_fail=True,
        expect_text="part_bytes",
    )

    # ── things that look like awaits and are not ─────────────────────────
    case(
        "an await in a comment or a string is not code",
        """
fn wire() {
    glib::spawn_future_local(async move {
        // secrets.store(&key).await would panic here.
        let message = "call .await on the runtime, not here";
        let answer = receiver.recv().await;
    });
}
""",
        should_fail=False,
    )

    case(
        "a crate with no crossing at all passes",
        "pub fn read_a_message() {}\n",
        should_fail=False,
    )

    # ── a char literal is not a string, and reading it as one loses the
    #    rest of the file (#1103) ────────────────────────────────────────
    #
    # `'"'` holds one double quote. Scanned as the start of a string literal
    # it inverts the quote parity of everything after it, so real code is
    # blanked as "string" and string contents are read as code -- and a
    # crossing past that point silently stops being seen.
    case(
        "a char literal holding a quote does not swallow the crossing after it",
        '''
fn escape(c: char) -> &'static str {
    match c {
        '"' => "&quot;",
        _ => "",
    }
}

fn submit() {
    glib::spawn_future_local(async move {
        secrets.store(&key, &password).await.unwrap();
    });
}
''',
        should_fail=True,
        expect_text="secrets.store",
    )

    case(
        "a lifetime is still not a char literal",
        '''
fn borrow<'a>(text: &'a str) -> &'a str {
    text
}

fn submit() {
    glib::spawn_future_local(async move {
        secrets.store(&key, &password).await.unwrap();
    });
}
''',
        should_fail=True,
        expect_text="secrets.store",
    )

    print()
    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
