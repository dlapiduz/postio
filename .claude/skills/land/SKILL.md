---
name: land
description: Commit finished Postio work correctly — verify the gates for your crate, stage only your own paths, write a conventional message whose body explains why, then close the bead. Use whenever a bead is done, and before going idle or ending a session.
---

# Land

The commit ritual, in order. Sessions get parts of this wrong silently — most
often by leaving a bead `in_progress` after its work is already committed.

## 1. Verify your crate is green

```bash
cargo test -p <your-crate>
cargo clippy -p <your-crate> --all-targets -- -D warnings
cargo fmt -p <your-crate>
python3 scripts/check-crate-boundaries.py
python3 scripts/check-no-personal-data.py
```

Per-crate, not `--workspace`: another session may be mid-TDD in a crate you do
not own, and requiring a green workspace would block you for their reasons.
Never format the whole workspace — it rewrites files they are editing.

If your crate is not green, keep working. Do not reach for a stash to get a
clean tree; that would take every session's changes, and the guard hook
refuses it.

## 2. Stage explicit paths

```bash
git add crates/<your-crate> Cargo.lock
```

Never stage everything — three other sessions have unfinished files in this
tree, and the guard hook refuses whole-tree staging. `Cargo.lock` churns
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
