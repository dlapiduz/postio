# Contract: the terminal surface

What a user (or a test) can rely on from `postio-tui`.

## Command lines

```text
postio-tui
```

- It takes no arguments and opens on the inbox.
- Exit `0` on quit. Exit `1` with one sentence on stderr for: the store open
  in another Postio window, keyring unavailable, or store refused.
- Environment: `NO_COLOR`, `COLORTERM`, `EDITOR`/`VISUAL`, `POSTIO_LOG`,
  `XDG_*`. No other variable changes behaviour.

## Commands and keys

- **Every command the registry offers the terminal is reachable** by its
  chord and in the command bar. The terminal is Focus (spec 007 C29): it is
  offered Focus's commands, less those that need pixels
  (`Requirement::Graphical`), plus the Markdown composer's own
  (`Requirement::Terminal`). An offered command with no key or no handler is
  a defect, and `tests/registry_parity.rs` enumerates the registry to prove
  there is none (SC-001).
- Chords are resolved by `postio_ui::keymap::Resolver` with `[keys]`
  overrides, as in the GTK interface.
- When a terminal lacks the kitty keyboard protocol, a chord it cannot
  deliver falls back to the command's registry `alternate_bindings`. The
  key map shows the chord that works *in this terminal*.
- While a text field has focus, printable keys insert text and never run a
  list command (FR-022a).

## Terminal equivalents of desktop-only affordances (FR-003)

| Desktop | Terminal |
|---|---|
| Detach the composer into its own window | Detach gives the composer the whole screen between the top bar and the bottom line. The same command id, the same draft |
| Drag a file onto the composer | Drop onto the terminal (arrives as a bracketed paste of its path) |
| Paste an image | Paste key: the clipboard image is read (arboard → wl-paste → xclip) |
| Rendered inline or remote images | `[image: alt · size]` placeholder, opens in the system viewer |
| The message's HTML, in app colours or on paper | Markdown-styled text in the terminal's colours (contracts/markdown.md); one treatment, so `O` is not offered |
| The row menu | The command bar, which acts on the row or selection that was focused when it opened |
| The main menu | Each item is a command with its key and a command-bar row |
| Settings window | A settings screen generated from `postio_ui::settings` sections |
| Toast | The bottom line, for the same 8 seconds |

## Layout

The window, its rows, the open message's frame, the reading pane, the
pickers and every other surface are drawn as
[spec 007's terminal.md](../../../../../specs/007-postio-focus/terminal.md) draws them. The
geometry is one module, `postio-tui::layout`, and every width in it comes
from the terminal's size alone. Below 50×12 the screen says "Terminal too
small: needs 50×12" and draws nothing else.

## Mouse

Everything the keyboard reaches also takes a click (terminal.md, "Mouse"):

| Gesture | Effect |
|---|---|
| Click a list row | Move the cursor there |
| Ctrl+click | Toggle that row in the selection |
| Shift+click | Extend the selection from the anchor |
| Click a row's answer, a strip item, a bulk-bar verb, a frame's step keys, close or action row | What its key does |
| Click a command-bar, folders-box or picker row, a tab or a reference | Choose it |
| Wheel | Scroll what is under the pointer, and nothing else |
| Click a link | Show its full target; a second click or Enter opens it (FR-014) |
| Click a fold marker | Expand or collapse it |
| Click an attachment | Offer open or save |
| Click in the composer | Place the cursor |
| Click outside a frame | Nothing; `Esc` and `✕` close it |

With `[tui].mouse = false`, or a terminal that reports nothing, clicks do
nothing and every other behaviour is unchanged.

## Paste and drop (composer)

`postio_ui::paste::classify` decides; the composer does exactly this:

- `File(path)`: attach it. The body is unchanged.
- `Unreadable { path, reason }`: a notice naming the file and the reason. The
  draft is unchanged.
- `Text(s)`: insert at the cursor (after `terminal::sanitize`).
- Paste key with an image on the clipboard: insert
  `![image](cid:<id>)` at the cursor, and attach inline.
- Paste key with no clipboard reachable: the notice "Clipboard unavailable
  here". Bracketed paste still works.

## Terminal state

- Raw mode, the alternate screen, mouse capture, bracketed paste and keyboard
  flags are restored on quit, on panic (a hook) and around `$EDITOR`.
- There is no suspend. Raw mode delivers `Ctrl+Z` as a key, and it is undo,
  as it is in every Postio app (specs/007-postio-focus, contracts/keymap.md).
- `$EDITOR` gets a `0600` file in `$XDG_RUNTIME_DIR/postio/`, which is deleted
  when the editor returns.

## Colour roles

As listed in data-model.md, overridable in `[tui.colors]`. The accent is used
only where the desktop uses it: markers, the action card's text, links, the
cursor's `▌` and the has-action toggle while it is on. Every state also has a
mark that is not a colour:

| State | Mark |
|---|---|
| Cursor | `▌` on each of the row's lines |
| Selected | `✓` in the mark column and the surface background (reversed without colour) |
| Unread | `●` and bold |
| Digest row | `≡` |
| Fired reminder | `↺` |
| Attachments | `⎘` |
| Has action, on | `⚑` toggle, reversed without colour |
| Marker | bold chip, italic quote |
| Notice | `✕` for a failure, `✓` for a success |

The cursor and a selection are different kinds of mark, so a cursor inside a
selection is still told apart from the rows around it; a row that is both
shows both.
