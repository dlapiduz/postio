# Contract: registry commands and the `[reader]` config section

Every command follows the four-edit path in `postio-core`:
1. the `command_ids!` entry;
2. the `Command` variant;
3. both conversion matches;
4. the `CommandSpec`.

After that, `docs/keybindings.md` is regenerated with `POSTIO_UPDATE_DOCS=1`,
and the golden `linux-bindings.txt` is updated **by hand**. Command ids are a
file format (Constitution II): they are chosen once, here.

| Id | Title | Default | Alternates | Contexts | Spec |
|---|---|---|---|---|---|
| `find_in_message` | Find in message | `mod+f` | — | message surfaces | FR-018 |
| `find_next` | Next match | `mod+g` | `F3` | message surfaces, while find is open | FR-018 |
| `find_previous` | Previous match | `mod+shift+g` | `shift+F3` | message surfaces, while find is open | FR-018 |
| `zoom_in` | Zoom in | `mod+plus` | `mod+equal`, `mod+KP_Add` | message surfaces | FR-021 |
| `zoom_out` | Zoom out | `mod+minus` | `mod+KP_Subtract` | message surfaces | FR-021 |
| `zoom_reset` | Actual size | `mod+0` | `mod+KP_0` | message surfaces | FR-021 |
| `darken_message` | Darken this message / Show as sent | `D` | — | message surfaces, dark theme, focused message is `Paper` or `Darkened` | FR-013a |
| `toggle_reader_view` | Reader view | `mod+shift+o` | `alt+o` | message surfaces | FR-031 |

**Why these keys.**
- `mod+f`, `mod+plus`, `mod+minus` and `mod+0` are the platform's
  conventions, and all are unbound today.
- `mod+shift+g` and `mod+shift+o` are bound today only in `Composer`
  (`Insert image…`, `Detach composer`). Compose takes over the reading pane,
  so the contexts never overlap. `command_registry.rs`'s conflict test is the
  arbiter: if it treats them as overlapping, the fallback is `shift+F3` as
  the primary for `find_previous`, and `mod+alt+o` for `toggle_reader_view`.
- `D` is unbound. `d` is taken, and shift-letter variants are this app's
  idiom (`A` archive, `J`/`K`).

**Properties.**
- None of these is destructive, and all have `Recovery::None`:
  `darken_message` and `toggle_reader_view` are self-inverse, and zoom has a
  reset.
- `darken_message`'s title reads **"Darken this message"** when the message
  is `Paper` and **"Show as sent"** when it is `Darkened`. That is the undo
  the spec requires.
- `view_original` (`mod+o`) keeps its meaning: it returns from Reader view.

**Mouse and gesture paths** (Constitution II: the mouse is excellent and
never required):
- Ctrl+scroll and pinch map onto `zoom_in` / `zoom_out`. They are not
  separate commands, and pinch snaps to a step on release.
- The zoom indicator's reset button dispatches `zoom_reset`.
- A context-menu entry in the reading pane offers `darken_message` and
  `toggle_reader_view`.

## `[reader]` in `config.toml`

```toml
[reader]
zoom = 100
```

| Key | Type | Default | Rule |
|---|---|---|---|
| `zoom` | integer percent | `100` | clamped to the nearest of 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300 on load; never an error |

- A zoom command writes the value back through the config writer.
- A live edit of the file applies to an open reader, as `[logging]` does.
- `docs/config.md` gains the row, and the config-doc drift test holds it.
