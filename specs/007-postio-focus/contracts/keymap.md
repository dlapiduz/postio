# Contract: one keymap for every app

`KEYS.md` is the default keymap for every app (spec, Clarifications). The
registry (`crates/postio-core/src/registry.rs`) is the one source: this file
is the table of its defaults that Focus's design relies on, and the
generated artefacts (`docs/keybindings.md`, the golden `linux-bindings.txt`)
follow the registry, not this file.

Legend:

- **All**: every app that has the surface.
- **Focus**: `Requirement::Focus`, offered only by Focus.
- **Graphical**: every app with a window.
- **Three-pane**: `Requirement::ThreePane`, which means "not Focus": the
  terminal and macOS (and the classic app until its removal) offer it, for
  their sidebar, panes, conversation rail and parts panel.
- `mod` is Ctrl on Linux and ⌘ on macOS (`crates/postio-config/src/keys.rs`).
- **The second layer**: `mod` chords that let the menu-shaped verbs sit in a
  menu bar, as alternates beside the keys below, never instead of them.
  `registry::offered_on` withholds a command from a platform with no surface
  for it (on macOS: `darken_message`, `detach_composer`, `next_scope`), and
  then it has no key there.

**Each app binds only what it offers.** An app builds its resolver with
`Resolver::from_commands_for(keymap, Frontend)`, so a key the keymap keeps
for Focus does nothing in another app: `y` does not answer an invitation in
the terminal.

## Message surfaces: List, Conversation, Reader

The open message is `Context::Reader`. Its fallback chain (Reader →
Conversation → List → Global) is what lets `j`/`k` step the list from inside
it.

| Key | Command id | Offered by | Notes |
|---|---|---|---|
| `j` / `Down` | `next_message` | All | |
| `k` / `Up` | `prev_message` | All | |
| `g g` / `G` | `first_message` / `last_message` | All | |
| `Return` / `Right` | `open_message` | All | |
| `x` | `toggle_selection` | All | |
| `J` / `K` | `extend_selection_down` / `_up` | All | List, reader and search; alternates `shift+Down` / `shift+Up` |
| `X` (alt `mod+a`) | `select_all` | All | A predicate over the view (C19) |
| `Escape` | `back` | All | Clears the selection in the list, closes a dialog |
| `]` / `[` | `next_in_conversation` / `prev_in_conversation` | All | Steps the open message through its thread. Alternates `alt+Down` / `alt+Up` |
| `e` / `E` / `f` | `reply` / `reply_all` / `forward` | All | Alternates `mod+r` / `mod+shift+r` / `mod+shift+f` |
| `a` | `archive` | All | Alternate `mod+shift+a` |
| `A` | `archive_thread` | All | In `Context::Digest` it archives the whole digest |
| `Delete` | `delete` | All | Moves to Trash, undoable (C12) |
| `s` | `snooze` | All | Opens the snooze picker |
| `B` | `unsnooze` | All | |
| `h` (alt `mod+h`) | `remind_if_no_reply` | Focus | `mod+h` is the composer's key, where `h` types |
| `r` | `toggle_read` | All | |
| `*` | `flag` | All | Toggles the flag; in Focus the row carries no mark (C13) |
| `l` | `add_label` | All | Opens the label picker |
| `m` | `move` | All | Opens the move picker |
| `mod+z` | `undo` | All | Cancels an open RSVP window first (research R9) |
| `c` | `compose` | All | Alternate `mod+n` |
| `y` / `Y` | `accept_invite` / `decline_invite` | Focus | |
| `U` | `unsubscribe` | All | Also in `Context::Digest` |
| `d` | `digest_rule` | Focus | From a message: a new sender rule. In a digest: edit its rule |
| `L` | `digest_like_this` | Focus | In the list, with a model configured |
| `v` | `view_source` | Focus | The raw RFC 822 message |
| `o` | `open_attachment_or_link` | Focus | A chooser over the message's links and parts, which also saves |
| `-` | `dismiss_marker` | Focus | |
| `t` / `n` | `capture_task` / `capture_note` | Focus | With a vault configured; also switch the capture sheet |
| `Left` | `prev_view` | All | |
| `F5` | `refresh` | All | |
| `Page_Down` / `Page_Up` (alt `space` / `shift+space`) | `scroll_reader_down` / `_up` | All | |

## Reading

| Key | Command id | Offered by | Notes |
|---|---|---|---|
| `O` | `switch_treatment` | Focus | Reader only: app colours or the original |
| `.` | `more_actions` | Focus | Reader only: the open message's More (Label, Move, Delete) |
| `i i` / `i a` | `show_images` / `always_show_images` | All | Once, or always for this sender |
| `mod+f`, `mod+g`, `mod+shift+g` | `find_in_message`, `find_next`, `find_previous` | All | Alternates `F3` / `shift+F3` |
| `mod+plus`, `mod+minus`, `mod+0` | `zoom_in`, `zoom_out`, `zoom_reset` | Graphical | Kept in `[reader] zoom` |
| `alt+d` | `darken_message` | Graphical | Withheld on macOS |
| `mod+o`, `mod+shift+o` | `view_original`, `toggle_reader_view` | All | |
| `F8` | `toggle_reading_pane` | Focus | List context, reached from the reader through its fallback: open messages beside the list or over it (`[focus] reading`) |

## Composer

| Key | Command id | Notes |
|---|---|---|
| `mod+Return` | `send` | Alternates `alt+s`, `alt+Return`, which a terminal delivers |
| `mod+shift+Return` | `schedule_send` | Opens the Send later picker. Alternate `alt+S` |
| `mod+shift+a` | `attach_file` | Alternate `alt+a` |
| `mod+h` | `remind_if_no_reply` | |
| `mod+shift+o` | `detach_composer` | Alternate `alt+o` |
| `mod+shift+c` | `copy_fields` | Cc and Bcc. Alternate `alt+c` |
| `mod+s` / `mod+d` | `save_draft` / `discard_draft` | |
| `mod+b`, `mod+i`, `mod+shift+k`, `mod+shift+7`, `mod+shift+8`, `mod+shift+9`, `mod+shift+g` | bold, italic, link, numbered and bullet lists, quote, image | Each with an `alt` alternate but bold |
| `mod+shift+x`, `mod+shift+y`, `mod+shift+m` | `cancel_send`, `retry_send`, `mark_sent` | Also in the list, for a draft being sent. Alternates `alt+x`, `alt+r`, `alt+m` |

## Going places

| Key | Command id | Offered by | Notes |
|---|---|---|---|
| `/` | `search` | All | Focus's command bar, open for mail search (C24). Alternate `alt+mod+f` |
| `mod+k` | `command_palette` | All | Focus's command bar in command mode, `>` typed (C24) |
| `g i` | `go_to_inbox` | All | |
| `g o` | `go_to_folders` | All | Focus's folders popover |
| `g t` | `go_to_drafts` | All | |
| `g s` | `go_to_sent` | All | |
| `g r` | `go_to_archive` | All | |
| `g z` | `go_to_snoozed` | All | |
| `g *` | `go_to_flagged` | All | |
| `g b` | `go_to_outbox` | Focus | The Outbox view of the first account |
| `g j` | `go_to_junk` | Focus | |
| `g #` | `go_to_trash` | Focus | |
| `g f` | `go_to_filtered` | Focus | |
| `g d` | `go_to_digest_rules` | Focus | |
| `g a` | `next_scope` | All | Withheld on macOS |
| `alt+1` … `alt+4` | `saved_search_1` … `saved_search_4` | All | The pinned `[filters]` entries, in their order |
| `!` | `toggle_has_action` | Focus | A punctuation alias in `postio-ui`'s keymap |

## Search: `Context::Search`

| Key | Command id | Offered by |
|---|---|---|
| `mod+s` | `save_search` | All |
| `mod+BackSpace` (alt `alt+BackSpace`, which a terminal delivers) | `back_to_words` | Focus |
| `O` | `toggle_result_order` | All |
| `Tab` | the bar's own chip navigation, not a registry command | All |

The `>` prefix in the command bar is the finder's mode prefix, not a key.

## Pickers: `Context::Picker`

| Key | Command id | Offered by |
|---|---|---|
| `1` … `4` | `picker_choose_1` … `picker_choose_4` | Focus |
| `Tab` | `picker_type_date` | Focus |
| `space` | `picker_toggle` | Focus |
| `Return` | `picker_confirm` | Focus |
| `Escape` | `back` | All |

## Digests and Filtered

| Context | Key | Command id | Offered by |
|---|---|---|---|
| `Digest` | `A` | `archive_thread` (the whole digest) | All |
| `Digest` | `d` | `digest_rule` (edit) | Focus |
| `Digest`, `Reader` | `D` | `stop_digesting_sender` | Focus |
| `Digest` | `U` | `unsubscribe` | All |
| `Digest` | `]` / `[` | `next_reference` / `prev_reference` | Focus |
| `Digest` | `Tab` | `toggle_digest_summary` | Focus |
| `Filtered` | `R` | `restore_filtered` | Focus |
| `Filtered` | `1` … `7` | `filtered_tab_1` … `filtered_tab_7` | Focus |
| `List` | `F` | `sweep_inbox`: shows a count, then acts | Focus |
| `Digest`, `Filtered` | `mod+z` | `undo` | All |

Both contexts fall back to Global only, so undo is bound in each: archiving
the whole digest and a restore from Filtered are undoable where they were
done (FR-116, FR-125).

## Obsidian: `Context::Capture`

| Key | Command id |
|---|---|
| `mod+p` | `capture_change_project` |
| `alt+s` | `capture_use_subject` |
| `mod+Return` (alt `alt+Return`) | `capture_write` |

## The application

| Key | Command id | Offered by |
|---|---|---|
| `?` | `cheat_sheet` (Focus's key map, screen 20) | All |
| `mod+comma` | `settings` (alt `alt+comma`) | All |
| `mod+shift+n` | `add_account` (alt `alt+n`) | All |
| `mod+e` | `edit_config`: `config.toml` in the person's editor | All |
| `mod+q` | `quit` (alt `mod+w`) | All |

## Accounts: `Context::Accounts`

Settings' Accounts section, with the keyboard on an account row. Offered by
every app.

| Key | Command id |
|---|---|
| `Return` | `toggle_account_enabled` |
| `Delete` | `remove_account` |
| `c` | `update_credential` |
| `r` | `rebuild_account_index` |
| `m` | `set_default_account` |
| `M` | `map_mailbox_role` |

## Three-pane surfaces keep their context keys

These surfaces exist only in the three-pane frontends (macOS, and the
classic app until its removal), each in its own context, so their keys do
not collide with the message surfaces':

- Global: `mod+b` toggles the sidebar; `tab` / `shift+tab` cycle the panes;
- Sidebar: `j`, `k`, `space`, `r`, `shift+Up`/`Down`, and `Delete` deletes a
  saved search;
- Parts: `p` opens it from the reader; `j`, `k`, `Return`, `s`, `S`, `x`,
  `H`;
- Conversation: `z` folds, `O` expands all, `I` toggles the rail.

## The terminal

- Raw mode delivers `ctrl+z` as a key, so undo is `ctrl+z`. The terminal
  does not suspend.
- Every key offered by All is deliverable by a terminal, `Delete`, `*`,
  `alt+1`–`alt+4`, `]`, `[` and `!` included; the terminal's parity test
  (`crates/postio-tui/tests/registry_parity.rs`) proves it.
- The terminal is Focus (C29): `Requirement::Focus` is met by it, and
  `Requirement::ThreePane` is not. It offers every Focus command but those
  that need pixels (`Requirement::Graphical`), and the Markdown composer's
  own (`Requirement::Terminal`).

## How it is tested

An enumeration across frontends (`Availability { frontend: Classic |
Terminal | Focus | Macos }`) asserts, for every app (SC-015):

- every command it offers has a default key;
- no key is bound to two commands in one context;
- a command's key is the same wherever it is offered.

Focus's `registry_parity` adds that every command Focus is offered reaches a
handler, and the key-map groups table is enumerated so every command Focus
offers has a group (research R4).
