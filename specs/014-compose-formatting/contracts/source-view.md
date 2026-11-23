# Contract: the Markdown source view (now)

The spec's "The Markdown source view" is the behaviour, and FR-023 to
FR-029 are the requirements. This file pins what the tests assert.

## The round trip (FR-024), `postio-body`

```text
normalize(d)                         merges adjacent Text and drops empty Text runs; idempotent
from_document(d) -> String
to_document(s)  -> Document
∀ d: normalize(d) == d ⇒ to_document(&from_document(&d)) == d      (excluding Block::Quoted, which the view keeps beside the text)
```

- A property test generates documents over every shape in `data-model.md`.
  This includes the awkward ones:
  - a break in a heading or a cell;
  - a break at the start or end of a paragraph;
  - an empty paragraph;
  - two adjacent lists of one kind;
  - leading and trailing spaces;
  - every punctuation character in text;
  - a backtick run in inline code;
  - a fence inside a code block.
- Golden spellings, each a unit case:

| Shape | Written as |
|---|---|
| Break where Markdown cannot place one | `<br>` |
| Empty paragraph | a line holding only `<br>` |
| Two adjacent bullet (or numbered) lists | an empty `<!-- -->` line between them |
| Leading or trailing space in a paragraph | `&#32;` |
| Remote image | `![alt](https://…)` |
| Carried quote | one line holding `U+E001` (the view draws the chip) |

## The caret (FR-027)

`from_document_mapped` and `to_document_mapped` return a `SourceMap`
(`data-model.md`). Unit cases cover every construct:

- **Every text character maps both ways and back to itself.**
  `to_document(to_source(p)) == p` for every text position `p`.
- **A source offset inside a marker** maps to the first text position of
  what it formats. The markers are `**`, `## `, `- [ ] `, `1. `, `> `, a
  fence line, a delimiter row and the `<br>` spellings.
- **The quote line** maps to the first position of the next block, or the
  end of the document.
- **Narrowed text** maps to the nearest surviving position before it.

## The surface (GTK `composer_markdown_source.rs`; Mac `ComposeModelTests`)

| Given | When | Then |
|---|---|---|
| the golden document | `mod+alt+s`, then `mod+alt+s` | the document is identical, and the draft's undo history has no new step |
| the caret in a struck word | `mod+alt+s` | the caret is on the same character in `~~…~~` |
| the view open | `mod+b` over a selection | `**` wraps it (`markdown_edit::wrap`) |
| the view open, a reply | Backspace on the quote chip's line | the chip goes whole, and `mod+z` brings it back |
| pasted text containing `U+E001` | paste | the character is gone |
| a ragged table typed | return to rich | a notice line names the dropped cells, with Undo |
| a session of edits in the view | return, then `mod+z` | the draft is as it was before the switch |
| saved in the view | reopen in GTK, on the Mac, in the terminal | the same text, in the Markdown view |
| the rich view | `mod+shift+s` | "Strikethrough is in the Markdown view for now · ctrl+alt+s", and no change |
| narrow, 1024 × 768 | open the view | the strip reads "Markdown · ctrl+alt+s" |

A storyboard, `storyboards/compose/markdown-source.toml`, films US7. It
is red on `main`, and `/ux-review` judges it.
