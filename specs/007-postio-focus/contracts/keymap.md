# Contract: one keymap for every app

The maintainer's answer (spec, Clarifications): `KEYS.md` becomes the default
for every app. This file is the resulting table: what each key does, which
command id it belongs to, and what moves out of the way. The registry
(`crates/postio-core/src/registry.rs`) stays the one source. This contract
describes the defaults the registry will hold, and generated artefacts
(`docs/keybindings.md`, the golden `linux-bindings.txt`) follow it.

Legend:

- **All**: every app that has the surface.
- **Focus**: `Requirement::Focus`, offered only by Focus.
- **Three-pane**: `Requirement::ThreePane`, offered by the apps with a
  sidebar and panes (classic, terminal, macOS) and not by Focus. It covers:
  - flag, the sidebar toggle, pane cycling and the parts panel (T029);
  - the conversation rail, and the folder-list, parts-panel and account-list
    keys (T166). `update_credential` stays with Focus, because its sign-in
    banner uses it.
- **New**: a new `CommandId`, and so a new `[keys]` name.
- `mod` is Ctrl on Linux and ⌘ on macOS (`crates/postio-config/src/keys.rs:191-221`).

## Message surfaces: List, Conversation, Reader

The Focus dialog is `Context::Reader`. Its fallback chain (Reader →
Conversation → List → Global) is what lets `j`/`k` step the list from inside
it.

| Key | Command id | Offered by | Change |
|---|---|---|---|
| `j` / `Down` | `next_message` | All | Unchanged |
| `k` / `Up` | `prev_message` | All | Unchanged |
| `g g` / `G` | `first_message` / `last_message` | All | Unchanged |
| `Return` / `Right` | `open_message` | All | The `l` alternate is dropped |
| `x` | `toggle_selection` | All | Unchanged |
| `J` / `K` | `extend_selection_down` / `_up` | All | Unchanged (list, reader, search) |
| `X` (alt `mod+a`) | `select_all` | All | Was `mod+a` |
| `Escape` | `back` | All | Unchanged. Clears the selection in the list, closes a dialog |
| `]` / `[` | `next_in_conversation` / `prev_in_conversation` | All | Were `J`/`K`, conversation only. Now also in the reader, where they step the dialog through the thread |
| `e` / `E` / `f` | `reply` / `reply_all` / `forward` | All | Unchanged |
| `a` | `archive` | All | Unchanged |
| `A` | `archive_thread` | All | Unchanged. In `Context::Digest` it archives the whole digest |
| `Delete` | `delete` | All | Was `d` |
| `s` | `snooze` | All | Was `b`. Opens the snooze picker; gains an `until` (R6) |
| `B` | `unsnooze` | All | Unchanged |
| `h` (alt `mod+h`) | `remind_if_no_reply` | Focus | **New.** `mod+h` is the composer's key, where `h` types |
| `r` | `toggle_read` | All | **New id, replaces `mark_unread` (`U`)** |
| `l` | `add_label` | All | Was `L`. Opens the label picker in Focus, and the finder's `+` mode in the classic app |
| `m` | `move` | All | Unchanged. Opens the move picker in Focus, and the finder's `#` mode in the classic app |
| `mod+z` | `undo` | All | Was `u`. Cancels an open RSVP window first (R9) |
| `c` | `compose` | All | Unchanged |
| `y` / `Y` | `accept_invite` / `decline_invite` | Focus | **New** |
| `U` | `unsubscribe` | All | Was `X` |
| `d` | `digest_rule` | Focus | **New.** From a message: the dialog for a new sender rule. In a digest: edit its rule and cadence |
| `v` | `view_source` | Focus | **New.** The raw RFC 822 message (R2). Focus-only for now: the other apps adopt it with a source view of their own, and its key is reserved for them |
| `o` | `open_attachment_or_link` | Focus | **New.** A chooser over the message's links and parts |
| `?` | `cheat_sheet` | All | Unchanged. Focus's key map (screen 20) |
| `*` | `flag` | Three-pane | Was `s`. Focus offers no flag verb (spec C13) |
| `Left` | `prev_view` | All | Was `h` (with `Left` as its alternate) |
| `alt+d` | `darken_message` | All graphical | Was `D` |

## Going places: Global, and the surfaces that go

| Key | Command id | Offered by | Change |
|---|---|---|---|
| `/` | `search` | All | Unchanged. Focus's command bar, open for mail search (spec C24) |
| `mod+k` | `command_palette` | All | Unchanged. Focus's command bar, open in command mode with `>` already typed, so only commands show (spec C24) |
| `g i` | `go_to_inbox` | All | Unchanged |
| `g o` | `go_to_folders` | All | **New id, replaces `focus_sidebar` (`g f`).** The classic app focuses its folder list, and Focus opens its folders popover |
| `g t` | `go_to_drafts` | All | Was `g d` |
| `g s` | `go_to_sent` | All | Was `g t` |
| `g r` | `go_to_archive` | All | **New** |
| `g z` | `go_to_snoozed` | All | **New** |
| `g *` | `go_to_flagged` | All | Was `g s` |
| `g f` | `go_to_filtered` | Focus | **New** |
| `g d` | `go_to_digest_rules` | Focus | **New** |
| `alt+1` … `alt+4` | `saved_search_1` … `saved_search_4` | All | **New.** The pinned `[filters]` entries, in their order |
| `!` | `toggle_has_action` | Focus | **New.** Needs a punctuation alias in `crates/postio-ui/src/keymap.rs:196-216` |

## Search: `Context::Search`

| Key | Command id | Offered by | Change |
|---|---|---|---|
| `mod+s` | `save_search` | All | Unchanged |
| `mod+BackSpace` | `back_to_words` | Focus | **New.** Returns from chips to the plain-English words |
| `O` | `toggle_result_order` | All | Was `o` |
| `Tab` | the search field's own chip navigation | All | Unchanged. It is not a registry command (#494) |

The `>` prefix in the command bar is the finder's mode prefix, not a key.
`Ctrl K` types it for you; `/` leaves the bar empty, for search. Both open the
bar in place, in the top bar's own field, with the results below it.

## Pickers: `Context::Picker` (new)

| Key | Command id | Offered by |
|---|---|---|
| `1` … `4` | `picker_choose_1` … `picker_choose_4` | Focus |
| `Tab` | `picker_type_date` | Focus |
| `space` | `picker_toggle` | Focus |
| `Return` | `picker_confirm` | Focus |
| `Escape` | `back` | All (unchanged; closes the picker) |

## Digests and Filtered (new contexts)

| Context | Key | Command id | Offered by |
|---|---|---|---|
| `Digest` | `A` | `archive_thread` (the whole digest) | All |
| `Digest` | `d` | `digest_rule` (edit) | Focus |
| `Digest`, `Reader` | `D` | `stop_digesting_sender` | Focus |
| `Digest`, `Reader` | `U` | `unsubscribe` | All |
| `Filtered` | `R` | `restore_filtered` | Focus |
| message surfaces | `-` | `dismiss_marker` | Focus (T118) |
| `List` | `F` | `sweep_inbox`: shows a count, then acts | Focus (T128) |
| `Filtered` | `1` … `7` | `filtered_tab_1` … `filtered_tab_7` | Focus |
| `Digest` | `]` / `[` | `next_reference` / `prev_reference` | Focus, milestone 2 |
| `Digest` | `Tab` | `toggle_digest_summary` | Focus, milestone 2 |
| `Digest`, `Filtered` | `mod+z` | `undo` | All |

Both contexts fall back to Global only, so undo is bound in each. Archiving
the whole digest is one undoable action (FR-125), and a restore from Filtered
is undoable (FR-116, T125). Each has to be undoable where it was done (T163).

**Each app binds only what it offers.** An app builds its resolver with
`Resolver::from_commands_for(keymap, Frontend)`. So a key the one keymap
keeps for Focus does nothing in the classic app, the terminal or macOS. For
example, `y` does not answer an invitation there (T029).

## Obsidian: `Context::Capture` (new, milestone 3)

| Key | Command id |
|---|---|
| `t` / `n` | `capture_task` / `capture_note` (in message surfaces) |
| `mod+p` | `capture_change_project` |
| `alt+s` | `capture_use_subject` |
| `mod+Return` | `capture_write` |

## Classic-only surfaces keep their context keys

These surfaces exist only in the classic app, and each has its own context,
so their keys are the classic ones:

- Sidebar: `j`, `k`, `space`, `r`, `shift+Up`/`Down`;
- Accounts: `Return`, `c`, `r`, `m`, `M`;
- Parts: `j`, `k`, `Return`, `s`, `S`, `x`, `H`;
- conversation-only: `z`, `O`, `I`.

Deleting a saved search (Sidebar) and removing an account (Accounts) move from
`d` to `Delete`, with the message verb, so "delete" has one key.

## The terminal

- **`mod+z`** becomes `ctrl+z`. Raw mode delivers it as a key
  (`crates/postio-tui/src/term.rs:184`), and today nothing is bound to it, so
  undo on `ctrl+z` works as soon as the default changes.
- **Suspend was never built,** although spec 005 says it was
  (`specs/005-tui-frontend/contracts/tui-surface.md:103-105`, `spec.md:360`,
  task T028). The same commit corrects those three claims.
- **Every key above offered by All is deliverable by a terminal:** `Delete`,
  `*`, `alt+1`–`alt+4`, `]`, `[` and `!`. The terminal's parity test
  (`crates/postio-tui/tests/registry_parity.rs:91-144`) proves it. Commands
  marked **Focus** are unmet by the terminal's `Availability`, so they are
  outside that test.

## What changes with it

These change in the same commit as the defaults:

- `crates/postio-core/tests/core_suite/command_registry.rs:75-164`: the tests
  that pin `u`, `h`, `l`, `g i`, `g d`, `g t` and `g s`;
- `crates/postio-core/src/registry.rs:2256-2275` and `:2327-2340`;
- `crates/postio-gtk/tests/logic_suite/keymap_defaults.rs:144-175`;
- the golden `linux-bindings.txt` (`crates/postio-core/tests/golden/`);
- `docs/keybindings.md`, regenerated with
  `POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`;
- the terminal's `app.rs` tests that press `s`, `d` and `X`;
- `docs/PRODUCT.md` §8 (`e` replies, `a` archives, `u` undoes, `J`/`K` walk a
  thread), which becomes the new sentence;
- `CLAUDE.md`'s line of keys (`e` reply, `a`/`A` archive, `u` undo, `J`/`K`
  walk a thread), which says `mod+z` for undo;
- the design canvas's key hints, which follow the registry at render time.

**How to test it.** An enumeration across frontends
(`Availability { frontend: Classic | Terminal | Focus | Macos }`) asserts, for
every app (SC-015):

- every command it offers has a default key;
- no key is bound to two commands in one context;
- a command's key is the same wherever it is offered.

The Focus key-map groups table (R4) is enumerated too. Every command Focus
offers has a group.
