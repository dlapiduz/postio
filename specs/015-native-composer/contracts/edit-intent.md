# Contract: `apply`

For `postio-body`, consumed by `postio-ui`, `postio-widgets`, the Mac
through `postio-ffi`, and (step 2) the WebView bridge.

## Signature

```rust
pub fn apply(document: &Document, selection: &Selection, intent: EditIntent) -> Edited;
```

Pure and total. No I/O, no clock, no toolkit.

## Rules

| Intent | Rule |
|---|---|
| `Insert(s)` over a range selection | Delete the range, then insert. Inline marks of the left end continue. |
| `SplitBlock` in a paragraph or heading | Split at the caret; the new block is a paragraph, except a heading after its end, which is a paragraph. |
| `SplitBlock` in a list item | Split the item; in an **empty last item**, leave the list (item becomes a paragraph after it). In an empty nested item, outdent. |
| `SplitBlock` in an empty quote child | Leave the quote. |
| `DeleteBackward` at offset 0 | Merge into the previous block; at the first item, un-list; at a quote's first child, un-quote. Never into or across a `Quoted` block, which is removed whole by a selection or by Backspace on its atomic unit. |
| `ToggleMark(m)` | If every character of the range has `m`, remove it; else add it. Over several blocks, each block's covered range. |
| `ToggleBlock(k)` | Toggle at the caret's block (or each block of the selection); lists and quotes wrap or unwrap, headings replace the kind. |
| `Indent` / `Outdent` | Nest the item under its previous sibling; un-nest. A first item cannot indent. |
| `Convert(c)` | Remove `trigger_len` characters before the caret, then apply the block or mark. Always `Join::New`. |
| `Paste(doc)` | Splice at the caret: inline content into the current block, blocks between. Always `Join::New`. |
| `CommitPreedit(s)` | As `Insert(s)`. `CancelPreedit` is `unchanged`. |
| Any intent while a preedit is set | Refused: `unchanged`, except the two preedit intents. |

## Guarantees

1. `parse(to_html(result.document)) == result.document`.
2. The selection is valid and, for a collapsed selection, so is its
   neighbour set (no caret in a `Quoted` block).
3. `result.changed` is the smallest range covering the difference; blocks
   outside it are equal to the input's.
4. Applying the inverse of the history step (`EditHistory::undo`) restores
   the input document. The selection after undo is stored on the step
   (`EditStep` gains `selection_before` and `selection_after`).
5. Nothing logs.

## Test placement (ADR 0049)

- Branch rules: `crates/postio-body/tests/body_suite/apply.rs`, one case
  per row above, plus the generated-document property for 1 to 3.
- Preedit and projection: `body_suite/projection.rs`.
- Matcher (which characters trigger what): `postio-ui`'s `editor::markdown`
  cases, unchanged in intent.
- Surfaces assert composition only: that a key reaches an intent and the
  patch reaches the buffer. They do not re-test the rows.
