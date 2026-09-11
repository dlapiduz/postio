# Contract: The Editor Document

**Producer**: `postio-ui::editor::document` (new, mirroring
`postio-ui::reader::document`)
**Consumers**: `postio-gtk`'s WebKit glue; the macOS frontend's equivalent

`postio-ui` is toolkit-free, so this contract is testable with no display and
both frontends inherit one answer. That is the same reason the reader's document
assembly lives there, stated in `postio-gtk/src/reader/view.rs`: *"one
implementation for every frontend… what remains in this file is webkit6 glue."*

## What the producer must supply

| Piece | Contract |
|---|---|
| **Stylesheet** | Typeface, size, foreground and background matching what the reader uses for a message body (FR-073) |
| **Ground colour** | The surface colour, resolved for the active scheme, so the view never shows a light frame before or after it draws (FR-074) |
| **Scheme** | Light and dark; a change is applied without reloading the document (FR-075) |
| **Quote treatment** | A quoted original is visually distinct from what the user is writing, in the same vocabulary the reader uses for a quote (FR-077) |
| **Density and text size** | Honours the application's settings, and stays usable at the narrow breakpoint (FR-076) |
| **Containment** | A quote's own styling is scoped so it cannot alter the user's text, a nested quote, or Postio's chrome (FR-078) |

## What the consumer must do

- Wrap the editing surface using this producer, not an inline `format!` of a
  document shell. Today `postio-gtk/src/editor.rs::seed` builds
  `<body contenteditable="true">` with a CSP and no stylesheet; that is the
  thing being replaced.
- Apply the ground colour to the view itself as well as the document, for the
  interval before the first paint — the same lesson as the reader's
  `paint_ground`.
- Re-apply on a scheme change **without** reloading: reloading the document
  would lose the caret and the undo history (FR-075).

## What is deliberately not shared with the reader

The CSP. The reader permits neither script nor `contenteditable`; the editor
requires both — `EDITOR_CSP` is `default-src 'none'; style-src 'unsafe-inline';
img-src postio-cid:`. Two documents, two policies, one set of tokens. Sharing
the policy would either loosen the reader or break the editor.

## How it is verified

- Unit tests in `postio-ui` over the generated document: it carries a
  stylesheet, the ground resolves in both schemes, the quote treatment is
  present, and a scheme change produces a different sheet from the same input.
- A `gtk_suite` case for what needs a display: the editing surface in dark mode
  is dark, and no light frame appears.
- The contract that cannot be unit-tested is "matches the reader" — assert it by
  deriving both from the same tokens rather than by comparing screenshots.
