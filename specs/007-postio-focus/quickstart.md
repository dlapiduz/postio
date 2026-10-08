# Quickstart: validating Postio Focus

How to prove the spec's acceptance scenarios, and where each proof lives. The
contracts say what is promised ([contracts/](./contracts/)). This file is how
to watch it hold.

## Prerequisites

- **A worktree of `feature/postio-focus`.** The system dependencies are in the
  README, and `scripts/install-nextest.sh` installs the pinned nextest.
- **A throwaway store.** This branch's store has Focus's tables. A store its
  migrations reach is carried forward; a store from a build that is not this
  one's ancestor (a `main` build's, say) is refused with "Start a fresh
  store", and going back to `main`'s build refuses this one. So never point a
  branch build at your everyday store. Give it its own directories:

  ```sh
  export XDG_DATA_HOME=$(mktemp -d) XDG_CONFIG_HOME=$(mktemp -d)
  cargo run -p postio-gtk
  ```

  `scripts/run-isolated.sh` builds a pinned commit of Postio with its own
  target directory and a throwaway store.

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
cargo nextest run -p postio-search                 # natural lowering, parse_when, the matcher
cargo nextest run -p postio-index --test index_suite       # the digest matcher's differential test
cargo nextest run -p postio-storage --test storage_suite   # Focus scopes: statements, rows, no scans (SC-002); migrations
cargo nextest run -p postio-core --test core_suite # the one keymap enumerated across apps (SC-015)
cargo nextest run -p postio-tui --test registry_parity     # the terminal still reaches every command
cargo nextest run -p postio-widgets --test widgets_suite   # the reader, composer and settings window
cargo nextest run -p postio-gtk --test focus_suite       # Focus's stories, asserted on the widget tree
scripts/check.sh                                   # boundaries, key hints, CSS, personal data
```

While the classic app still builds, its suites stay green too (SC-010).

**Nightly:** SC-011's first classification pass over a 100,000-message store
and the excerpt locator's floors run under `--profile nightly`, marked
`POSTIO-MEASUREMENT:`.

## Screens: compare each with its PNG

```sh
for s in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
  cargo run -p postio-gtk --example shot -- /tmp/focus-$s.png "$s"
done
cargo run -p postio-gtk --example shot -- /tmp/focus-02.png 02 dark
```

`shot` draws a named screen from a seeded demo store: the corpus-derived
mailbox, plus markers, digests and filter decisions written through the host.
Beyond the drawn screens it knows the row menu (26), the body treatments
(27–31), the composer with an attachment (32), the fresh-store page (33), the
reading pane (34–36) and Settings (40–43). It exits non-zero with `NO IMAGE
WAS WRITTEN` when there was nothing to draw. The sizes and the compositor it
wants are in [screens.md](./screens.md), "Rendering them".

**Read every PNG back,** beside its reference in the maintainer's
`Design/postio-focus-design/screens/` (and `Design/focus-message-dialog/` for
the open message), in light and dark.

**Record every difference** in [screens.md](./screens.md): one row per
screen, each difference with its reason (FR-095, SC-009). The ones the spec
decided are its table *Where the inputs disagree* (C1–C27).

## By hand: what a person should see

### 1. One store, any app (User Story 11)

1. Open the terminal (`cargo run -p postio-tui`) on the throwaway store and
   archive a message. Leave it open.
2. Start Focus. It says "Postio is already open in another window. Close it to
   open Postio here." and changes nothing.
3. Close the terminal, then choose Try again. Focus opens, and the archived
   message is not in its inbox.

### 2. Triage (User Stories 1 and 5)

1. Press `j` and `k`: only the cursor moves. Press `x` on three rows, then
   `a`.
2. The toast reads "Archived 3 messages". Press `mod+z`, and all three
   return.
3. Press `s` and choose `2`. The row leaves. Snoozed mail is under `g z`.
4. Press `h`, type "tue 9am" and press `Enter`. The reminder is set.
5. Press `!`. Only marked rows remain, and the strip says "Showing N of M".
   Press `!` again to restore the full inbox.

### 3. Open and come back (User Story 2)

1. Select two rows, move the cursor to a third, and press `Enter`. The
   message opens over the list.
2. Press `j`: the next message shows. Press `[`: the thread steps back.
   Press `v`: the raw source shows. On an HTML newsletter, press `O`: the
   body switches between app colours and the original.
3. Press `Esc`. You are on the same row, and the two rows are still
   selected.
4. Press `F8`, then `Enter`: the message opens beside the list, and `j`/`k`
   step it. `F8` again puts it back over the list.

### 4. Write (User Story 3)

1. Press `E` on a thread. The recipients, "Re:" and the thread's labels are
   filled in.
2. Set "Remind if no reply" and send (`mod+Return`) with the network off.
   The message waits in the Outbox (`g o` lists it) and leaves when the
   network returns.

### 5. The bar (User Story 4)

1. Press `/` and type "the invoice ada sent last month". Chips appear.
2. Press `Tab` to move through the chips, and `mod+BackSpace` to return to
   the words.
3. Press `mod+k` and type "arch": only commands are listed.
4. Press `g o`, type "rec" and press `Enter`. Receipts' mail is listed.

### 6. States (User Story 6)

1. Turn the network off. The "You're offline" banner appears. Triage still
   works, and the changes queue.
2. Give the account a wrong password. The "Can't sign in" banner appears, with
   Update password….
3. A fresh account shows the first-sync banner with its progress, and the
   inbox says it is syncing, not that it is empty.

### 7. Invitations (User Story 8)

1. From a second test account, send an invitation to the throwaway account.
2. The row shows Invite, the event's time, and Accept `y` / Decline `Y`.
3. Press `y` and then `mod+z` within ten seconds: nothing is sent.
4. Press `y` again and wait. One reply leaves.

### 8. Filtering (User Story 9)

1. With Focus running, have a list mail from a sender you have never written
   to arrive. It goes to Filtered (`g f`) with its reason, and never appears
   in the inbox.
2. Press `R` on it. It returns to the inbox, and `config.toml`'s
   `[focus.filter] never` gains the sender.
3. With the terminal open instead, the same mail lands in its inbox. When
   Focus next opens, the mail moves to Filtered.

### 9. Digests (User Story 10)

1. Press `d` on a newsletter and choose weekly, due in two minutes. The
   preview count shows.
2. A new message from that sender does not reach the inbox. `g d` lists it
   under the rule.
3. When the rule comes due, one digest row appears. `Enter` lists its
   messages, and `A` archives them all as one undo.

### 10. One keymap (User Story 7)

1. In the terminal, `s` snoozes, `ctrl+z` undoes, and `u` does nothing.
2. Override `archive = "w"` under `[keys]` and save. In Focus the key map,
   the command bar and the Archive button show `w`, and that override frees
   nothing else.

### 11. Privacy (FR-150, SC-016)

1. With no `[focus.model]` in the config, run Focus through a triage session
   and check that nothing connects to a model runtime:
   `ss -tnp | grep -E '11434|8080'` shows nothing. The automated check is a
   test that fails on any connection attempt.
2. Check that the logs carry ids, counts and outcomes only: set
   `POSTIO_LOG=debug` and search the output for a subject line.

### 12. Settings

1. Press `mod+comma`. Settings opens over the list. Change a signature, close
   it with `Esc`, and compose: the new signature is there.
2. Press `mod+e` anywhere: `config.toml` opens in your editor, and what you
   save there takes effect without a restart.
