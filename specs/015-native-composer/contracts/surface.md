# Contract: what a native surface does, on either toolkit

Both the `GtkTextView` and the `NSTextView` surface implement this. It is
the conformance table's frame (FR-017): a script of inputs, and the
`Document` expected after each.

## Input to intent

| Input | Intent |
|---|---|
| Text commit | `Insert` (or `CommitPreedit` when a preedit was open) |
| Preedit update | no intent: set `Projection.preedit`, redraw the hole |
| Enter, Shift+Enter | `SplitBlock`, `InsertBreak` |
| Backspace, Delete, with Ctrl or Alt for word | `DeleteBackward`/`DeleteForward` |
| Tab, Shift+Tab in a list item | `Indent`, `Outdent` (else the focus moves) |
| A registry composer command | its intent, via `postio_ui` |
| Paste, drop | `Paste(parse(...))` |
| Cut | copy, then `DeleteBackward` over the range |
| Undo, Redo | `EditHistory::undo`/`redo`; the surface repaints the changed range |

## Output

After each intent the surface calls `Projection::patch(changed, new)` and
applies the returned edits to the buffer or storage, with edit signals
blocked, then sets the selection from `Edited.selection` via `offset_of`.

## Must hold

- No buffer mutation outside the patch (a test asserts the signal-blocked
  count equals the patch's operation count).
- The saved draft is `Document::to_html` and `body_markdown`, never the
  buffer's text.
- Accessible names on every atomic unit.
- No network, no script, no scheme handler.
