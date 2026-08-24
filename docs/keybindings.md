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

While you are typing, single-key bindings do not fire. Only `Escape`,
the function keys, and chords holding `Ctrl`, `Alt` or `Super` reach a
command from inside a text field.

## Bindings

| Keys | Command | Where | Undo | Id |
|---|---|---|---|---|
| `j` or `Down` | Next message | List, thread, reader, search |  | `next_message` |
| `k` or `Up` | Previous message | List, thread, reader, search |  | `prev_message` |
| `g g` | First message | List, thread, reader, search |  | `first_message` |
| `G` | Last message | List, thread, reader, search |  | `last_message` |
| `Return` or `l` or `Right` | Open message | List, thread, search |  | `open_message` |
| `x` | Toggle selection | List, thread, reader, search |  | `toggle_selection` |
| `J` or `shift+Down` | Extend selection down | List, thread, reader, search |  | `extend_selection_down` |
| `K` or `shift+Up` | Extend selection up | List, thread, reader, search |  | `extend_selection_up` |
| `ctrl+a` | Select all | List, thread, reader, search |  | `select_all` |
| `h` or `Left` | Previous view | List, thread, reader |  | `prev_view` |
| `Escape` | Back | Everywhere |  | `back` |
| `t` | Show thread | List, reader |  | `thread` |
| `e` | Reply | List, thread, reader |  | `reply` |
| `E` | Reply to all | List, thread, reader |  | `reply_all` |
| `f` | Forward | List, thread, reader |  | `forward` |
| `a` | Archive | List, thread, reader | Undoable | `archive` |
| `A` | Archive thread | List, thread, reader | Undoable | `archive_thread` |
| `d` | Delete | List, thread, reader | Undoable | `delete` |
| `m` | Move to… | List, thread, reader | Undoable | `move` |
| `s` | Flag | List, thread, reader | Undoable | `flag` |
| `U` | Mark unread | List, thread, reader | Undoable | `mark_unread` |
| `L` | Add label… | List, thread, reader | Undoable | `add_label` |
| `/` | Search | List, thread, reader |  | `search` |
| `c` | Compose | List, thread, reader |  | `compose` |
| `ctrl+Return` | Send | Composer | Undoable | `send` |
| `ctrl+s` | Save draft | Composer |  | `save_draft` |
| `ctrl+d` | Discard draft | Composer | Asks first | `discard_draft` |
| `ctrl+shift+a` | Attach file… | Composer |  | `attach_file` |
| `ctrl+shift+o` | Detach composer | Composer |  | `detach_composer` |
| `u` | Undo | List, thread, reader |  | `undo` |
| `ctrl+k` | Command palette | Everywhere |  | `command_palette` |
| `?` | Keyboard shortcuts | List, thread, reader |  | `cheat_sheet` |
| `ctrl+comma` | Settings | Everywhere |  | `settings` |
| `ctrl+e` | Edit configuration | List, thread, reader |  | `edit_config` |
| `ctrl+b` | Toggle sidebar | List, thread, reader |  | `toggle_sidebar` |
| `g f` | Focus the folder list | List, thread, reader, search |  | `focus_sidebar` |
| `j` or `Down` | Next folder | Folder list |  | `next_folder` |
| `k` or `Up` | Previous folder | Folder list |  | `prev_folder` |
| `F5` or `R` | Refresh | List, thread, reader |  | `refresh` |
