---
name: land
description: Commit finished Postio work correctly — verify the gates for your crate, stage only your own paths, write a conventional message whose body explains why, then close the bead. Use whenever a bead is done, and before going idle or ending a session.
---

# Land

The commit ritual, in order. Sessions get parts of this wrong silently — most
often by leaving a bead `in_progress` after its work is already committed.

## 1. Verify your crate is green — once, here

This is the **only** place the gates need to run. Formatting in particular is
not verification: run it here, not after every edit.

```bash
rustfmt --edition 2024 $(git diff --name-only HEAD -- '*.rs') \
  && cargo clippy -p <your-crate> --all-targets -- -D warnings \
  && cargo test   -p <your-crate> \
  && python3 scripts/check-crate-boundaries.py \
  && python3 scripts/check-no-personal-data.py crates/<your-crate>
```

One chained command, in that order: format first so clippy and the tests see
the final bytes, and the chain stops at the first failure.

**Pass your crate path to the personal-data check.** Unscoped it scans every
tracked file, so in a shared tree it fails on another session's uncommitted
edits and tells you nothing about your own work. CI runs it unscoped; you
should not.

**Per-crate, not `--workspace`.** A workspace test compiles and runs all nine
crates including GTK, and serialises on the shared target directory while other
sessions are building — it is the largest wall-clock cost in this project.
Another session may also be mid-TDD in a crate you do not own, so a red
workspace is usually their business, not yours. Verify your crate; let CI prove
the workspace.

**Format the files you touched, not the crate.** `cargo fmt -p <crate>`
reformats *every* file in that crate — including a file another session has
open and uncommitted. That already happened once: a composer session ran
`cargo fmt -p postio-gtk` and churned whitespace through the settings session's
in-flight test file. Nothing was lost, but their diff got noise they did not
write. `rustfmt --edition 2024 <files>` touches only what you name; the command
above derives that list from your own changes.

`cargo fmt --all --check` and `-p <crate> --check` are read-only and safe. It is
only the writing forms that reach into other people's files.

If your crate is not green, keep working. Do not reach for a stash to get a
clean tree; that would take every session's changes with it.

## 2. Stage explicit paths

```bash
git add crates/<your-crate> Cargo.lock
```

Never stage everything — other sessions have unfinished files in this tree.

**And never `git commit -- <path>`.** That form bypasses the index and commits
the *working tree* version of those paths, so anything you had half-written in
them goes in too. A session did exactly this and produced an intermediate
commit that would not build in isolation. Stage first, then commit with no
pathspec:

```bash
git add crates/<your-crate> Cargo.lock
git commit          # no -- <path>, no -a
``` `Cargo.lock` churns
constantly and is a resolved superset; staging it alongside your crate is
expected and merges fine.

## 3. Write the message

```
<type>(<scope>): <summary>

Why this change exists, wrapped at 72 columns.

Refs: postio-abc
```

- **scope** is the crate without its `postio-` prefix
- **summary**: imperative, lower case, no trailing period, at most 50 chars
- **body explains why** — the diff already says what. Write one whenever the
  change encodes a decision, a trade-off, or a non-obvious constraint. If you
  learned something the hard way, that belongs here; it is the most valuable
  thing in the commit.
- `Refs:` on every commit; `Closes:` when the commit completes the bead

One bead per commit. If the subject needs an "and", it is two commits.

## 4. Close the bead

```bash
bd close <id> --reason="<what you did>" --suggest-next
```

Do this immediately after committing. A bead left claimed makes other sessions
skip work that is actually available, and once a session ends nobody can tell
finished work from abandoned work without reading diffs.

If you are stopping mid-bead, commit anyway — marked as work in progress, with
the bead left open:

```
feat(storage): begin the operation queue drainer

Work in progress -- retry classification is not implemented yet.

Refs: postio-abc
```

Uncommitted work is unprotected work.

## 5. Never push

Commits are standing-authorised in this repository; pushes are not. Do not
push, add a remote, or rewrite history unless the user asks in this session.
The guard hook refuses all three.
