# Keyboard reference

<!-- Generated from `postio-core`'s command registry and the one
box's mode table by `crates/postio-ui/tests/ui_suite/keybindings_doc.rs`.
Do not edit by hand:
change the registry and run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`. -->

Every command below is also in the `Ctrl+K` palette and the `?` cheat
sheet of every app that offers it, because all three are generated
from one table.

Every Postio app has this one keymap. A command only some apps offer
says which in the Where column, and keeps its key free in the others.
`docs/PRODUCT.md` §8 records how the keys were settled; this table is
the registry.

## Rebinding

Every binding is overridable from the `[keys]` section of
`config.toml`, keyed by the command id in the last column:

```toml
[keys]
archive = "w"
first_message = "g g"
```

A chord joins modifiers to a key with `+` (`ctrl+k`); a sequence
separates chords with a space (`g g`). Shift is written into the
character, so `A` is what you get by holding shift — `a` and `A` are
different bindings. An override that cannot be used, or that collides
with a key already taken in the same place, is reported in the settings
panel and the command keeps its default.

`mod` is the primary accelerator: Control here, Command on macOS.
Every default above uses it, which is why the same `config.toml`
means the same thing on both. Writing `ctrl` instead pins the
binding to Control everywhere.

While you are typing, single-key bindings do not fire. Only `Escape`,
the function keys, and chords holding `Ctrl`, `Alt` or `Super` reach a
command from inside a text field.

## Bindings

| Keys | Command | Where | Undo | Id |
|---|---|---|---|---|
| `j` or `Down` | Next message | List, conversation, reader, search, digest, Filtered view |  | `next_message` |
| `k` or `Up` | Previous message | List, conversation, reader, search, digest, Filtered view |  | `prev_message` |
| `g g` | First message | List, conversation, reader, search |  | `first_message` |
| `G` | Last message | List, conversation, reader, search |  | `last_message` |
| `Return` or `Right` | Open message | List, conversation, search, digest, Filtered view |  | `open_message` |
| `x` | Toggle selection | List, conversation, reader, search |  | `toggle_selection` |
| `J` or `shift+Down` | Extend selection down | List, reader, search |  | `extend_selection_down` |
| `K` or `shift+Up` | Extend selection up | List, reader, search |  | `extend_selection_up` |
| `X` or `ctrl+a` | Select all | List, conversation, reader, search |  | `select_all` |
| `Left` | Previous view | List, conversation, reader |  | `prev_view` |
| `Escape` | Back | Everywhere |  | `back` |
| `alt+o` | Toggle result order | Search |  | `toggle_result_order` |
| `]` or `alt+Down` | Next message in conversation | Conversation, reader |  | `next_in_conversation` |
| `[` or `alt+Up` | Previous message in conversation | Conversation, reader |  | `prev_in_conversation` |
| `z` | Fold or unfold this message | Conversation |  | `toggle_fold` |
| `ctrl+o` | View original | List, conversation, reader |  | `view_original` |
| `ctrl+shift+o` or `alt+o` | Reader view | List, conversation, reader |  | `toggle_reader_view` |
| `alt+d` | Darken this message | List, conversation, reader (not the terminal) |  | `darken_message` |
| `O` | Show original or app colours | Reader (not macOS) |  | `switch_treatment` |
| `ctrl+f` | Find in message | List, conversation, reader |  | `find_in_message` |
| `ctrl+g` or `F3` | Next match | List, conversation, reader |  | `find_next` |
| `ctrl+shift+g` or `shift+F3` | Previous match | List, conversation, reader |  | `find_previous` |
| `ctrl+plus` or `ctrl+equal` or `ctrl+KP_Add` | Zoom in | List, conversation, reader (not the terminal) |  | `zoom_in` |
| `ctrl+minus` or `ctrl+KP_Subtract` | Zoom out | List, conversation, reader (not the terminal) |  | `zoom_out` |
| `ctrl+0` or `ctrl+KP_0` | Actual size | List, conversation, reader (not the terminal) |  | `zoom_reset` |
| `O` or `ctrl+shift+e` | Expand all | Conversation |  | `expand_all` |
| `I` | Hide or show the conversation rail | Conversation (macOS) |  | `toggle_rail` |
| `e` or `ctrl+r` | Reply | List, conversation, reader, composer |  | `reply` |
| `E` or `ctrl+shift+r` | Reply to all | List, conversation, reader, composer |  | `reply_all` |
| `f` or `ctrl+shift+f` | Forward | List, conversation, reader, composer |  | `forward` |
| `a` or `ctrl+shift+a` | Archive | List, conversation, reader | Undoable | `archive` |
| `A` | Archive thread | List, conversation, reader, digest | Undoable | `archive_thread` |
| `Delete` | Delete | List, conversation, reader | Undoable | `delete` |
| `m` | Move to… | List, conversation, reader | Undoable | `move` |
| `*` | Flag | List, conversation, reader | Undoable | `flag` |
| `r` | Mark read or unread | List, conversation, reader | Undoable | `toggle_read` |
| `s` | Snooze | List, conversation, reader | Undoable | `snooze` |
| `B` | Unsnooze | List, conversation, reader | Undoable | `unsnooze` |
| `h` or `ctrl+h` | Remind if no reply… | List, conversation, reader, composer (not macOS) | Undoable | `remind_if_no_reply` |
| `l` | Add label… | List, conversation, reader | Undoable | `add_label` |
| `y` | Accept invitation | List, conversation, reader (not macOS) | Undo briefly | `accept_invite` |
| `Y` | Decline invitation | List, conversation, reader (not macOS) | Undo briefly | `decline_invite` |
| `d` | Digest rule… | List, conversation, reader, digest (not macOS) |  | `digest_rule` |
| `D` | Stop digesting this sender | Reader, digest (not macOS) | Undoable | `stop_digesting_sender` |
| `v` | View source | List, conversation, reader (not macOS) |  | `view_source` |
| `o` | Open attachment or link… | List, conversation, reader (not macOS) |  | `open_attachment_or_link` |
| `-` | Dismiss marker | List, conversation, reader (not macOS) | Undoable | `dismiss_marker` |
| `.` | More actions | Reader (not macOS) |  | `more_actions` |
| `F8` | Read beside the list or over it | List (not macOS) |  | `toggle_reading_pane` |
| `/` or `alt+ctrl+f` | Search | List, conversation, reader, search, folder list |  | `search` |
| `ctrl+s` | Save search as folder | Search |  | `save_search` |
| `ctrl+BackSpace` or `alt+BackSpace` | Back to words | Search (not macOS) |  | `back_to_words` |
| `c` or `ctrl+n` | Compose | List, conversation, reader |  | `compose` |
| `ctrl+Return` or `alt+s` or `alt+Return` | Send | Composer | Undo briefly | `send` |
| `ctrl+shift+Return` or `alt+S` | Schedule send… | Composer |  | `schedule_send` |
| `ctrl+s` | Save draft | Composer |  | `save_draft` |
| `ctrl+d` | Discard draft | Composer | Asks first | `discard_draft` |
| `ctrl+shift+m` or `alt+m` | Mark as sent | List, composer |  | `mark_sent` |
| `ctrl+shift+y` or `alt+r` | Retry send | List, composer |  | `retry_send` |
| `ctrl+shift+x` or `alt+x` | Cancel send | List, composer |  | `cancel_send` |
| `ctrl+shift+a` or `alt+a` | Attach file… | Composer |  | `attach_file` |
| `ctrl+shift+o` or `alt+o` | Detach composer | Composer |  | `detach_composer` |
| `ctrl+shift+c` or `alt+c` | Cc and Bcc | Composer |  | `copy_fields` |
| `ctrl+shift+g` or `alt+g` | Insert image… | Composer |  | `insert_image` |
| `ctrl+shift+e` or `alt+e` | Edit in external editor | Composer (terminal) |  | `edit_externally` |
| `ctrl+shift+p` or `alt+p` | Toggle preview | Composer (terminal) |  | `toggle_preview` |
| `ctrl+b` | Bold | Composer |  | `bold` |
| `ctrl+i` or `alt+i` | Italic | Composer |  | `italic` |
| `ctrl+shift+8` or `alt+8` | Bulleted list | Composer |  | `bullet_list` |
| `ctrl+shift+7` or `alt+7` | Numbered list | Composer |  | `numbered_list` |
| `ctrl+shift+k` or `alt+k` | Insert link… | Composer |  | `insert_link` |
| `ctrl+shift+9` or `alt+9` | Quote block | Composer |  | `quote_block` |
| `ctrl+z` | Undo | List, conversation, reader, account list, digest, Filtered view |  | `undo` |
| `ctrl+k` | Command palette | Everywhere |  | `command_palette` |
| `?` | Keyboard shortcuts | List, conversation, reader |  | `cheat_sheet` |
| `ctrl+comma` or `alt+comma` | Settings | Everywhere |  | `settings` |
| `ctrl+shift+n` or `alt+n` | Add account | Everywhere |  | `add_account` |
| `ctrl+e` | Edit configuration | List, conversation, reader |  | `edit_config` |
| `ctrl+q` or `ctrl+w` | Quit Postio | Everywhere |  | `quit` |
| `i i` | Show remote images | List, conversation, reader |  | `show_images` |
| `i a` | Always show images from this sender | List, conversation, reader |  | `always_show_images` |
| `U` | Unsubscribe from this list | List, conversation, reader, digest |  | `unsubscribe` |
| `ctrl+b` | Toggle sidebar | List, conversation, reader, folder list (macOS) |  | `toggle_sidebar` |
| `g o` | Go to folders | List, conversation, reader, search |  | `go_to_folders` |
| `g i` | Go to inbox | List, conversation, reader, search, folder list, digest, Filtered view |  | `go_to_inbox` |
| `g t` | Go to drafts | List, conversation, reader, search, folder list |  | `go_to_drafts` |
| `g s` | Go to sent | List, conversation, reader, search, folder list |  | `go_to_sent` |
| `g *` | Go to flagged | List, conversation, reader, search, folder list |  | `go_to_flagged` |
| `g r` | Go to archive | List, conversation, reader, search, folder list |  | `go_to_archive` |
| `g z` | Go to snoozed | List, conversation, reader, search, folder list |  | `go_to_snoozed` |
| `g b` | Go to outbox | List, conversation, reader, search, folder list (not macOS) |  | `go_to_outbox` |
| `g j` | Go to junk | List, conversation, reader, search, folder list (not macOS) |  | `go_to_junk` |
| `g #` | Go to trash | List, conversation, reader, search, folder list (not macOS) |  | `go_to_trash` |
| `g f` | Go to Filtered | List, conversation, reader, search, folder list (not macOS) |  | `go_to_filtered` |
| `g d` | Go to digest rules | List, conversation, reader, search, folder list (not macOS) |  | `go_to_digest_rules` |
| `alt+1` | Saved search 1 | List, conversation, reader, search, folder list |  | `saved_search_1` |
| `alt+2` | Saved search 2 | List, conversation, reader, search, folder list |  | `saved_search_2` |
| `alt+3` | Saved search 3 | List, conversation, reader, search, folder list |  | `saved_search_3` |
| `alt+4` | Saved search 4 | List, conversation, reader, search, folder list |  | `saved_search_4` |
| `!` | Show only what has an action | List (not macOS) |  | `toggle_has_action` |
| `tab` | Next pane | List, conversation, reader, folder list (macOS) |  | `cycle_pane` |
| `shift+tab` | Previous pane | List, conversation, reader, folder list (macOS) |  | `cycle_pane_back` |
| `j` or `Down` | Next folder | Folder list (macOS) |  | `next_folder` |
| `k` or `Up` | Previous folder | Folder list (macOS) |  | `prev_folder` |
| `space` | Expand or collapse folder | Folder list (macOS) |  | `toggle_folder` |
| `r` | Rename saved search | Folder list (macOS) |  | `rename_saved_search` |
| `shift+Up` | Move saved search up | Folder list (macOS) |  | `move_saved_search_up` |
| `shift+Down` | Move saved search down | Folder list (macOS) |  | `move_saved_search_down` |
| `Delete` | Delete saved search | Folder list (macOS) | Asks first | `delete_saved_search` |
| `Return` | Enable or disable account | Account list |  | `toggle_account_enabled` |
| `Delete` | Remove account | Account list | Undoable | `remove_account` |
| `c` | Update account credential | Account list |  | `update_credential` |
| `r` | Rebuild search index | Account list |  | `rebuild_account_index` |
| `m` | Set as default account | Account list |  | `set_default_account` |
| `M` | Map mailbox role | Account list | Undoable | `map_mailbox_role` |
| `g a` | Next scope | List, folder list |  | `next_scope` |
| `F5` | Refresh | List, conversation, reader |  | `refresh` |
| `p` | Show message parts | Reader (macOS) |  | `open_parts` |
| `j` or `Down` | Next part | Parts panel (macOS) |  | `next_part` |
| `k` or `Up` | Previous part | Parts panel (macOS) |  | `prev_part` |
| `Return` | Open part | Parts panel (macOS) |  | `open_part` |
| `s` | Save part | Parts panel (macOS) |  | `save_part` |
| `S` | Save all parts | Parts panel (macOS) |  | `save_all_parts` |
| `x` | Open part externally | Parts panel (macOS) |  | `open_part_externally` |
| `H` | Render part once | Parts panel (macOS) |  | `render_part_once` |
| `Page_Down` or `space` | Scroll reading pane down | List, conversation, reader |  | `scroll_reader_down` |
| `Page_Up` or `shift+space` | Scroll reading pane up | List, conversation, reader |  | `scroll_reader_up` |
| `1` | Choose option 1 | Picker (not macOS) |  | `picker_choose_1` |
| `2` | Choose option 2 | Picker (not macOS) |  | `picker_choose_2` |
| `3` | Choose option 3 | Picker (not macOS) |  | `picker_choose_3` |
| `4` | Choose option 4 | Picker (not macOS) |  | `picker_choose_4` |
| `tab` | Type a date | Picker (not macOS) |  | `picker_type_date` |
| `space` | Toggle option | Picker (not macOS) |  | `picker_toggle` |
| `Return` | Confirm | Picker (not macOS) |  | `picker_confirm` |
| `]` | Next reference | Digest (not macOS) |  | `next_reference` |
| `[` | Previous reference | Digest (not macOS) |  | `prev_reference` |
| `tab` | Summary or messages | Digest (not macOS) |  | `toggle_digest_summary` |
| `R` | Restore to inbox | Filtered view (not macOS) | Undoable | `restore_filtered` |
| `1` | Reason 1 | Filtered view (not macOS) |  | `filtered_tab_1` |
| `2` | Reason 2 | Filtered view (not macOS) |  | `filtered_tab_2` |
| `3` | Reason 3 | Filtered view (not macOS) |  | `filtered_tab_3` |
| `4` | Reason 4 | Filtered view (not macOS) |  | `filtered_tab_4` |
| `5` | Reason 5 | Filtered view (not macOS) |  | `filtered_tab_5` |
| `6` | Reason 6 | Filtered view (not macOS) |  | `filtered_tab_6` |
| `7` | Reason 7 | Filtered view (not macOS) |  | `filtered_tab_7` |
| `F` | Filter what is in the inbox… | List (not macOS) | Undoable | `sweep_inbox` |
| `t` | Capture a task… | List, conversation, reader, capture sheet (not macOS) |  | `capture_task` |
| `n` | Capture a note… | List, conversation, reader, capture sheet (not macOS) |  | `capture_note` |
| `ctrl+p` | Change project | Capture sheet (not macOS) |  | `capture_change_project` |
| `alt+s` | Use the subject instead | Capture sheet (not macOS) |  | `capture_use_subject` |
| `ctrl+Return` or `alt+Return` | Add to the vault | Capture sheet (not macOS) |  | `capture_write` |
| `L` | Digest mail like this | List (not macOS) |  | `digest_like_this` |

## The one box

`/` opens one box in the header, and it answers more than one
question. Typing searches mail; a character typed into an empty box
chooses what else to ask, and is absorbed into a marker on the field
rather than staying in the query. Backspace at the start gives the
mode back and keeps what was typed.

| Typed | What it does |
|---|---|
| *(nothing)* | Search all mail |
| `>` | Run a command |
| `#` | Go to a folder |
| `@` | Find a correspondent |
| `+` | Add a label |
