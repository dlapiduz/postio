# Keyboard reference

<!-- Generated from `postio-core`'s command registry by
`crates/postio-core/tests/keybindings_doc.rs`. Do not edit by hand:
change the registry and run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-core`. -->

Every command below is also in the `Ctrl+K` palette and the `?` cheat
sheet, because all three are generated from one table.

Bindings come from the design canvas — `e` replies, not `r`.
`docs/PRODUCT.md` §8 records that resolution; this table is the registry.

## Rebinding

Every binding is overridable from the `[keys]` section of
`config.toml`, keyed by the command id in the last column:

```toml
[keys]
archive = "y"
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
| `j` or `Down` | Next message | List, conversation, reader, search |  | `next_message` |
| `k` or `Up` | Previous message | List, conversation, reader, search |  | `prev_message` |
| `g g` | First message | List, conversation, reader, search |  | `first_message` |
| `G` | Last message | List, conversation, reader, search |  | `last_message` |
| `Return` or `l` or `Right` | Open message | List, conversation, search |  | `open_message` |
| `x` | Toggle selection | List, conversation, reader, search |  | `toggle_selection` |
| `J` or `shift+Down` | Extend selection down | List, conversation, reader, search |  | `extend_selection_down` |
| `K` or `shift+Up` | Extend selection up | List, conversation, reader, search |  | `extend_selection_up` |
| `ctrl+a` | Select all | List, conversation, reader, search |  | `select_all` |
| `h` or `Left` | Previous view | List, conversation, reader |  | `prev_view` |
| `Escape` | Back | Everywhere |  | `back` |
| `o` | Toggle result order | Search |  | `toggle_result_order` |
| `ctrl+o` | View original | List, conversation, reader |  | `view_original` |
| `e` | Reply | List, conversation, reader, composer |  | `reply` |
| `E` | Reply to all | List, conversation, reader, composer |  | `reply_all` |
| `f` | Forward | List, conversation, reader, composer |  | `forward` |
| `a` | Archive | List, conversation, reader | Undoable | `archive` |
| `A` | Archive thread | List, conversation, reader | Undoable | `archive_thread` |
| `d` | Delete | List, conversation, reader | Undoable | `delete` |
| `m` | Move to… | List, conversation, reader | Undoable | `move` |
| `s` | Flag | List, conversation, reader | Undoable | `flag` |
| `U` | Mark unread | List, conversation, reader | Undoable | `mark_unread` |
| `b` | Snooze | List, conversation, reader | Undoable | `snooze` |
| `B` | Unsnooze | List, conversation, reader | Undoable | `unsnooze` |
| `L` | Add label… | List, conversation, reader | Undoable | `add_label` |
| `/` | Search | List, conversation, reader |  | `search` |
| `ctrl+s` | Save search as folder | Search |  | `save_search` |
| `c` | Compose | List, conversation, reader |  | `compose` |
| `ctrl+Return` | Send | Composer | Undoable | `send` |
| `ctrl+shift+Return` | Schedule send… | Composer |  | `schedule_send` |
| `ctrl+s` | Save draft | Composer |  | `save_draft` |
| `ctrl+d` | Discard draft | Composer | Asks first | `discard_draft` |
| `ctrl+shift+m` | Mark as sent | List, composer |  | `mark_sent` |
| `ctrl+shift+a` | Attach file… | Composer |  | `attach_file` |
| `ctrl+shift+o` | Detach composer | Composer |  | `detach_composer` |
| `ctrl+b` | Bold | Composer |  | `bold` |
| `ctrl+i` | Italic | Composer |  | `italic` |
| `ctrl+shift+8` | Bulleted list | Composer |  | `bullet_list` |
| `ctrl+shift+7` | Numbered list | Composer |  | `numbered_list` |
| `ctrl+shift+k` | Insert link… | Composer |  | `insert_link` |
| `ctrl+shift+9` | Quote block | Composer |  | `quote_block` |
| `u` | Undo | List, conversation, reader, account list |  | `undo` |
| `ctrl+k` | Command palette | Everywhere |  | `command_palette` |
| `?` | Keyboard shortcuts | List, conversation, reader |  | `cheat_sheet` |
| `ctrl+comma` | Settings | Everywhere |  | `settings` |
| `ctrl+shift+n` | Add account | Everywhere |  | `add_account` |
| `ctrl+e` | Edit configuration | List, conversation, reader |  | `edit_config` |
| `ctrl+b` | Toggle sidebar | List, conversation, reader |  | `toggle_sidebar` |
| `g f` | Focus the folder list | List, conversation, reader, search |  | `focus_sidebar` |
| `tab` | Next pane | List, conversation, reader, folder list |  | `cycle_pane` |
| `shift+tab` | Previous pane | List, conversation, reader, folder list |  | `cycle_pane_back` |
| `j` or `Down` | Next folder | Folder list |  | `next_folder` |
| `k` or `Up` | Previous folder | Folder list |  | `prev_folder` |
| `space` | Expand or collapse folder | Folder list |  | `toggle_folder` |
| `r` | Rename saved search | Folder list |  | `rename_saved_search` |
| `shift+Up` | Move saved search up | Folder list |  | `move_saved_search_up` |
| `shift+Down` | Move saved search down | Folder list |  | `move_saved_search_down` |
| `d` | Delete saved search | Folder list | Asks first | `delete_saved_search` |
| `Return` | Enable or disable account | Account list |  | `toggle_account_enabled` |
| `d` | Remove account | Account list | Undoable | `remove_account` |
| `c` | Update account credential | Account list |  | `update_credential` |
| `g a` | Next scope | List, folder list |  | `next_scope` |
| `F5` or `R` | Refresh | List, conversation, reader |  | `refresh` |
| `p` | Show message parts | Reader |  | `open_parts` |
| `j` or `Down` | Next part | Parts panel |  | `next_part` |
| `k` or `Up` | Previous part | Parts panel |  | `prev_part` |
| `Return` | Open part | Parts panel |  | `open_part` |
| `s` | Save part | Parts panel |  | `save_part` |
| `S` | Save all parts | Parts panel |  | `save_all_parts` |
| `x` | Open part externally | Parts panel |  | `open_part_externally` |
| `H` | Render part once | Parts panel |  | `render_part_once` |
| `Page_Down` or `space` | Scroll reading pane down | List, conversation, reader |  | `scroll_reader_down` |
| `Page_Up` or `shift+space` | Scroll reading pane up | List, conversation, reader |  | `scroll_reader_up` |
