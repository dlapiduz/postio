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
{ git diff --name-only --diff-filter=d HEAD -- 'crates/<your-crate>/*.rs'
  git ls-files --others --exclude-standard  -- 'crates/<your-crate>/*.rs'
} | xargs -r rustfmt --edition 2024 \
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
write. `rustfmt --edition 2024 <files>` touches only what you name.

**The pathspec is what makes "the files you touched" true.** Naming your crate
in both halves is not tidiness — without it this command has the very bug it
exists to avoid. `git diff --name-only HEAD` lists what is dirty in the *whole
tree*, and in a shared checkout that is every session's work. An earlier
version of this block omitted the pathspec, and a session ran it over 272 lines
of someone else's loose work; nothing was damaged, but only because those files
happened to be formatted already. Use the same paths you will pass to
`git commit --only`.

Three details in the command carry weight. It lists untracked files too —
`git diff HEAD` alone silently skips a brand-new test file, which is exactly
how unformatted code has reached a commit here before. `--diff-filter=d` drops
deletions, which `rustfmt` cannot open. And `xargs -r` makes the empty case a
no-op — bare `rustfmt` with no file arguments waits on stdin, so without it a
session with nothing to format appears to hang.

`cargo fmt --all --check` and `-p <crate> --check` are read-only and safe. It is
only the writing forms that reach into other people's files.

If your crate is not green, keep working. Do not reach for a stash to get a
clean tree; that would take every session's changes with it.

## 2. Commit your paths, atomically

```bash
git commit --only crates/<your-crate> Cargo.lock -m "..."
```

**Not `git add` then `git commit`.** The index is *shared*: those are two steps,
and anything another session stages in between lands in your commit. That has
happened three times here — one commit absorbed another session's compose work,
another absorbed an unrelated keyring fix. `--only` commits exactly the paths
you name and leaves everyone else's staged work untouched, which makes it a
single atomic step.

Never `git add -A`, `git add .`, or `git commit -a`.

**New files need `git add` first.** `--only` diffs tracked paths, so it cannot
introduce a file git has never seen. Add just your new files, then commit with
`--only` over everything you are landing:

```bash
git add crates/<your-crate>/src/new_thing.rs        # new files only
git commit --only crates/<your-crate> Cargo.lock -m "..."
```

Adding two named new files is a far smaller race window than `git add -A`, and
`--only` still scopes the commit to your paths.

**`--only` does not protect a file someone else is editing.** It commits the
*working-tree* version of every path you name, so if another session has
uncommitted changes in a file you list, those changes go into your commit. That
happened on `postio-app/src/engine.rs` and briefly broke HEAD. `--only` saves
you from what others have **staged elsewhere**; it cannot save you from a file
you both touch.

So before committing, look at what you are about to take:

```bash
git status --porcelain <your paths>      # every line here must be yours
git diff <your paths>                    # if you did not write it, do not commit it
```

If a shared file has someone else's work in it, wait, or commit your other
paths and leave that one.

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

## 4. Is it actually reachable?

Before closing a bead that builds a **surface** — a widget, a pane, a command,
a view — answer this: can a person reach it in the running app?

This has gone wrong three times here. Commands resolved through the registry,
the keymap, the palette and the selection model and then hit a no-op handler
(`postio-agr`). The whole search UI was built and tested and nothing fed it
(`postio-1ag`). The list and sidebar were finished before anything connected
them to the store. Each time the tests passed, the bead looked done, and the
epic percentage said the feature existed when it did not.

So: **either wire it, or file the wiring bead before you close** — and say so
in the closing reason. A green test suite proves the widget works. It does not
prove the application does.

If you cannot wire it because another session owns the file, that is a good
reason to file rather than force it. Name the exact calls the wiring needs, so
whoever takes it does not have to re-derive them.

## 5. Close the bead

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

## 6. Never push

Commits are standing-authorised in this repository; pushes are not. Do not
push, add a remote, or rewrite history unless the user asks in this session.
The guard hook refuses all three.
