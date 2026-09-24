# Contract: the terminal surface

What a user (or a test) can rely on from `postio-tui`.

## Command lines

```text
postio-tui [--scope <query>] [--compose [mailto:…]]
postio-tui --version
```

- Exit `0` on quit. Exit `1` with one sentence on stderr for: the store open
  in another Postio window, keyring unavailable, or store refused.
- A `mailto:` argument opens a composer, the same as GTK's `HANDLES_OPEN`.
- Environment: `NO_COLOR`, `COLORTERM`, `EDITOR`/`VISUAL`, `POSTIO_LOG`,
  `XDG_*`. No other variable changes behaviour.

## Commands and keys

- **Every command in `postio_core::registry::all()` is reachable** by its
  chord and in the palette. There is no terminal-only command, and no
  registry command is missing (SC-001; enumeration test).
- Chords are resolved by `postio_ui::keymap::Resolver` with `[keys]`
  overrides, as in GTK.
- When a terminal lacks the kitty keyboard protocol, a chord it cannot
  deliver falls back to the command's registry `alternate_bindings`. The
  cheat sheet shows the chord that works *in this terminal*.
- While a text field has focus, printable keys insert text and never run a
  list command (FR-022a).

## Terminal equivalents of desktop-only affordances (FR-003)

| Desktop | Terminal |
|---|---|
| Pop-out composer window (`ctrl+shift+o`) | The composer moves to a tab of its own. The same command id, the same draft. |
| Drag a file onto the composer | Drop onto the terminal (arrives as a bracketed paste of its path) |
| Paste an image | Paste key: the clipboard image is read (arboard → wl-paste → xclip) |
| Rendered inline or remote images | `[image: alt · size]` placeholder, opens in the system viewer |
| WebKit reader | Markdown-styled text (contracts/markdown.md) |
| Context menu | The palette, filtered to the focused item's context |
| Settings window | A settings screen generated from `postio_ui::settings` sections |
| Desktop notification click raises the window | The notification is delivered by GTK when it is connected (protocol §Notifications) |

## Layout

Panes by terminal width, following ADR 0024's rule that width decides what is
*shown*, never what was *asked for*:

| Columns | Shown |
|---|---|
| ≥ 140 | sidebar · list · reader |
| 90–139 | list · reader (sidebar on request, as an overlay) |
| 50–89 | one pane at a time: list ⇄ reader/composer |
| < 50 or < 12 rows | "Terminal too small: needs 50×12", and nothing else |

The exact widths are one table in `postio-tui::layout`, as GTK's are in
`postio-gtk::shell`.

Inside the panes, the canvas's PLATE 1b in a terminal: a top bar across the
width (the search field, which *is* the search once one is open, and the
cheat sheet's and compose's keys from the keymap); the sidebar headed by each
account's address, with the sync state at its foot; list rows of two lines
(marks, sender and time over subject and preview); the reader headed by its
subject and `from → to, cc · date`, its body wrapped to the pane, and the
reply and archive keys at its foot; dim rules between panes; and a status line
of notices on the left and the count on the right. The palette and the cheat
sheet are rounded, titled overlays.

## Mouse

| Gesture | Effect |
|---|---|
| Click a sidebar row | Open that scope |
| Click a list row | Move the cursor there; the reader follows |
| Ctrl+click | Toggle that row in the selection |
| Shift+click | Extend the selection from the anchor |
| Wheel over a pane | Scroll *that* pane only |
| Click a link | Show its full target; a second click or Enter opens it (FR-014) |
| Click a fold marker | Expand or collapse it |
| Click a placeholder or attachment | Offer open or save |
| Drag a pane divider | Resize; the width persists in window state |
| Click in the composer | Place the cursor |

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
  flags are restored on quit, on panic (a hook), on `Ctrl+Z` (SIGTSTP; re-armed
  on SIGCONT, with a full redraw) and around `$EDITOR`.
- `$EDITOR` gets a `0600` file in `$XDG_RUNTIME_DIR/postio/`, which is deleted
  when the editor returns.

## Colour roles

As listed in data-model.md, overridable in `[tui.colors]`. Unread, flagged,
selected and focus each also have a non-colour mark:

| State | Mark |
|---|---|
| Unread | `●` column and bold |
| Flagged | `⚑` column |
| Attachments | `⎘` column |
| Cursor | `▌` bar and the `Surface` tint across both lines (reverse without colour) |
| Selected | `✓` in a gutter stripe of `Selection`'s own colour (reverse without colour) |
| Open folder | `▌` bar and the `Surface` tint |
| Notice | `✕` for a failure, `✓` for a success |

The cursor and a selection are different kinds of mark, in different
colours, so a cursor inside a selection is still told apart from the rows
around it; a row that is both shows both.
