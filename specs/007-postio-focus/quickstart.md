# Quickstart: validating Postio Focus

How to prove the spec's acceptance scenarios, and where each proof lives. The
contracts say what is promised ([contracts/](./contracts/)). This file is how
to watch it hold.

## Prerequisites

- **A worktree of `feature/postio-focus`.** The system dependencies are in the
  README, and `scripts/install-nextest.sh` installs the pinned nextest.
- **A throwaway store.** This branch changes the store's schema (the
  data-model). A build from it refuses an existing store, and that store then
  resyncs into a fresh one. The same thing happens when you go back to
  `main`'s build. So never point a branch build at your everyday store. Give
  it its own directories:

  ```sh
  export XDG_DATA_HOME=$(mktemp -d) XDG_CONFIG_HOME=$(mktemp -d)
  cargo run -p postio-focus
  ```

- **Fixtures only, in anything committed.** Every fixture, screenshot and
  test uses reserved domains and fictional people (spec FR-152). The Focus
  PNGs in the maintainer's checkout carry a real name, and are compared
  locally, never committed until they are re-rendered.

## Automated: what the landing gate and CI run

**Between edits,** the fast tier: `scripts/test-fast.sh`.

**Before landing,** the suites the diff touches:

```sh
cargo nextest run -p postio-classify               # rules, guards, the detector (SC-013's corpus gate)
cargo nextest run -p postio-calendar               # invitations: requests, updates, cancellations, zones
cargo nextest run -p postio-search                 # natural lowering, parse_when, the digest matcher's differential test
cargo nextest run -p postio-storage --test storage_suite   # Focus scopes: statements, rows, no scans (SC-002)
cargo nextest run -p postio-core --test core_suite # the one keymap enumerated across apps (SC-015)
cargo nextest run -p postio-tui --test registry_parity     # the terminal still reaches every command
cargo nextest run -p postio-focus --test focus_suite       # Focus's stories, asserted on the widget tree
cargo nextest run -p postio-app --test app_suite   # the classic app, unchanged but for keys (SC-010)
cargo nextest run -p postio-gtk --test gtk_suite   # the reader and composer after the move
scripts/check.sh                                   # boundaries, key hints, CSS, personal data
```

**Nightly:** SC-011's first classification pass over a 100,000-message store
runs under `--profile nightly`, marked `POSTIO-MEASUREMENT:`.

## Screens: compare each with its PNG

```sh
for s in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17 18 19 20; do
  cargo run -p postio-focus --example shot -- /tmp/focus-$s.png "$s"
done
cargo run -p postio-focus --example shot -- /tmp/focus-02.png 02 dark
```

`shot` draws a named screen from a seeded demo store: the corpus-derived
mailbox, plus markers, digests and filter decisions written through the host.
It exits non-zero with `NO IMAGE WAS WRITTEN` when there was nothing to draw.

**Read every PNG back,** beside its reference in the maintainer's
`Design/postio-focus-design/screens/`, in light and dark.

**Record every difference** in `specs/007-postio-focus/screens.md`: one row
per screen, each difference with its reason (spec FR-095, SC-009). The known
ones are listed in the spec's table *Where the inputs disagree* (C1–C23).

## By hand: what a person should see

### 1. One store, either app (User Story 11)

1. Open the classic app on the throwaway store and archive a message. Leave it
   open.
2. Start Focus. It says "Postio is already open in another window. Close it to
   open Postio here." and changes nothing.
3. Close the classic app, then choose Try again. Focus opens, and the archived
   message is not in its inbox.

### 2. Triage (User Stories 1 and 5)

1. Press `j` and `k`: only the cursor moves. Press `x` on three rows, then
   `a`.
2. The toast reads "Archived 3 messages". Press `Ctrl+Z`, and all three
   return.
3. Press `s` and choose `2`. The row leaves. Snoozed mail is under `g z`.
4. Press `h`, type "tue 9am" and press `Enter`. The reminder is set.
5. Press `!`. Only marked rows remain, and the strip says "Showing N of M".
   Press `!` again to restore the full inbox.

### 3. Open and come back (User Story 2)

1. Select two rows, move the cursor to a third, and press `Enter`. The dialog
   opens.
2. Press `j`: the next message shows. Press `[`: the thread steps back.
   Press `v`: the raw source shows.
3. Press `Esc`. You are on the same row, and the two rows are still
   selected.

### 4. Write (User Story 3)

1. Press `E` on a thread. The recipients, "Re:" and the thread's labels are
   filled in.
2. Set "Remind if no reply" and send with the network off. The message waits
   in the Outbox and leaves when the network returns.

### 5. The bar (User Story 4)

1. Press `/` and type "the invoice ada sent last month". Chips appear.
2. Press `Tab` to move through the chips, and `Ctrl+Backspace` to return to
   the words.
3. Type `>arch`: only commands are listed.
4. Press `g o`, type "rec" and press `Enter`. Receipts' mail is listed.

### 6. States (User Story 6)

1. Turn the network off. The "You're offline" banner appears. Triage still
   works, and the changes queue.
2. Give the account a wrong password. The "Can't sign in" banner appears, with
   Update password….
3. A fresh account shows the first-sync banner with its progress.

### 7. Invitations (User Story 8)

1. From a second test account, send an invitation to the throwaway account.
2. The row shows Invite, the event's time, and Accept `y` / Decline `Y`.
3. Press `y` and then `Ctrl+Z` within ten seconds: nothing is sent.
4. Press `y` again and wait. One reply leaves.

### 8. Filtering (User Story 9)

1. With Focus running, have a list mail from a sender you have never written
   to arrive. It goes to Filtered (`g f`) with its reason, and never appears
   in the inbox.
2. Press `R` on it. It returns to the inbox, and `config.toml`'s
   `[focus.filter] never` gains the sender.
3. With the classic app open instead, the same mail lands in its inbox. When
   Focus next opens, the mail moves to Filtered.

### 9. Digests (User Story 10)

1. Press `d` on a newsletter and choose weekly, due in two minutes. The
   preview count shows.
2. A new message from that sender does not reach the inbox. `g d` lists it
   under the rule.
3. When the rule comes due, one digest row appears. `Enter` lists its
   messages, and `⇧A` archives them all as one undo.

### 10. One keymap (User Story 7)

1. In the classic app, `s` snoozes, `Ctrl+Z` undoes, and `u` does nothing.
2. In the terminal, `Ctrl+Z` undoes.
3. Override `archive = "y"` under `[keys]` and save. In every app the key map,
   the command bar and the Archive button show `y`, and in Focus that override
   frees nothing else.

### 11. Privacy (FR-150, SC-016)

1. With no `[focus.model]` in the config, run Focus through a triage session
   and check that nothing connects to a model runtime:
   `ss -tnp | grep -E '11434|8080'` shows nothing. The automated check is a
   test that fails on any connection attempt.
2. Check that the logs carry ids, counts and outcomes only: set
   `POSTIO_LOG=debug` and search the output for a subject line.
