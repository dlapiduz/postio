# It hands you other worktrees' artifacts, and the compile error then names a file that is correct

*Archived 2026-09-14: the war stories of the shared cargo target directory, and the record of #178 resolving it on 2026-08-25; every worktree has had its own `target/` since, seeded by reflink on claim (#1102), so the symptom cannot recur in a tree claimed after that.*

This is not contention and not a stale cache — it was
demonstrated end to end while landing #82.

`cargo test -p postio-app` in the `issue-82` worktree failed with:

```
error[E0308]: mismatched types
   --> crates/postio-gtk/src/reader/view.rs:438:13
    |
438 |             sanitized.remote_blocked,
    |             ^^^^^^^^^^^^^^^^^^^^^^^^ expected `bool`, found `u32`
```

That worktree's own `postio-body/src/sanitize.rs` declares
`pub remote_blocked: bool`, and its `postio-gtk` is right to expect a `bool`.
The `u32` exists in exactly one place on this machine: the `issue-58`
worktree, where another session is mid-refactor turning that flag into a
count. So `postio-gtk` from one worktree was compiled against `postio-body`
from another, through the shared `CARGO_TARGET_DIR` that CLAUDE.md tells every
session to set.

The same run had produced a second symptom earlier —
`no variant ... named DetachComposer found for enum postio_core::CommandId`,
against a `command.rs` that declares it four times — from worktrees still on
an older `main`. Both are the same fault wearing different clothes.

**The worst instance so far did not look like a build problem at all.** It
looked like a broken `main`. `postio-gtk`'s
`cheatsheet::tests::the_sections_are_the_ones_the_registry_actually_uses`
failed *deterministically* — every run, filtered to that one test, single
threaded, in a fresh worktree and in the shared checkout — reporting an extra
"Thread" section holding two commands, "Unread only" and "Toggle order".
Neither string existed anywhere in the worktree under test. Both existed in
the `issue-61` worktree, where a session was adding them. The test binary had
linked *that* `postio-core`.

Two things make this the dangerous shape. It was **repeatable**, so the usual
"run it again" tell was absent. And it presented as exactly the case
CLAUDE.md's CI section says to respond to by pulling `ready`
off every open issue — a disruptive, repository-wide stop, triggered by a
regression that did not exist. Rebuilding in a private `CARGO_TARGET_DIR`
passed first time.

So add one step before believing a red `main`: **grep the sibling worktrees
for the symbol in the error.**

```sh
grep -rl "<symbol from the failure>" ~/src/postio-worktrees/*/crates/
```

If it turns up in a worktree that is not yours, the error is about the build.

Three things follow, and the third is the one that costs time:

- **`cargo build --workspace` succeeding proves nothing about the next run.**
  It depends on what the other sessions happened to have built by then.
- **Building the failing crate alone is often clean**, because a narrower
  build reuses less. `cargo clippy -p postio-gtk` passed while
  `cargo clippy -p postio-app` failed on `postio-gtk`, minutes apart.
- **Do not go looking for the bug.** Check `pgrep -c 'cargo|rustc'` and
  whether the type in the error message exists in a *sibling worktree*
  (`grep -r <symbol> ~/src/postio-worktrees/*/crates/`). If it does, the
  error is about the build, not the code.

The reliable fix is a `CARGO_TARGET_DIR` of your own for that run. It costs a
full duplicate build, which is why it is not the default — but see the next
entry before choosing where to put it. Tracked as #178.

## Resolved 2026-08-25 (#178): worktrees stopped sharing a target directory

The mechanism was never pinned down, but the effect was proven
twice (a `bool`-vs-`u32` type error against a declaration that was correct;
`CommandId::DetachComposer` missing against a `command.rs` that declares it),
and every diagnosis of it cost the wrong kind of time. The replacement:
each worktree builds into its own `target/` and `RUSTC_WRAPPER=sccache`
carries the third-party compilation cost once per machine — sccache keys on
exact compiler inputs, so it cannot serve a sibling's artifact. Numbers that
shaped the choice: the shared directory had grown to ~157 GB (du,
hardlink-inflated) against 99 GB free, so nobody "migrates" by copying —
new claims simply start private, the cache warms as sessions build what
they touch, and the legacy directory is reclaimed when the last session
sharing it is gone. `issue-claim.sh` now also creates `target/tmp` in the
fresh worktree, because `.cargo/config.toml` points TMPDIR there and its
absence made every `tempfile::tempdir()` in a fresh worktree fail with
NotFound — three sessions hit that in one day. The interim tell above stays
true for anyone still on the shared directory.
