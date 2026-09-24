# Contract: Markdown ↔ Document ↔ HTML

Three new functions in `postio_body::markdown`, and what each promises. The
`Document` (`crates/postio-body/src/document.rs`) is unchanged.

```text
from_html(sanitized_html: &str) -> String         // reading: HTML → CommonMark
to_document(markdown: &str) -> Document           // writing: Markdown → Document
from_document(doc: &Document) -> String           // reopening a GTK draft
```

The dialect is CommonMark plus GFM tables, strikethrough and autolinks
(pulldown-cmark options `TABLES | STRIKETHROUGH | TASKLISTS`, with a
`linkify` pass for bare URLs).

## `to_document`: writing

| Markdown | Document | Notes |
|---|---|---|
| paragraph | `Block::Paragraph` | |
| hard break (two spaces or `\`) | `Inline::Break` | soft breaks become spaces |
| `**x**` / `__x__` | `Inline::Strong` | |
| `*x*` / `_x_` | `Inline::Emphasis` | |
| `` `x` `` | `Inline::Code` | |
| `# ` `## ` `### ` | `Block::Heading{1..3}` | |
| `####`+ | `Block::Heading{3}` | the Document's own narrowing (`parse.rs`) |
| `- ` `* ` `+ ` / `1. ` | `Block::List{ordered}` | nests; a start number other than 1 is dropped |
| `> ` | `Block::Quote` | a quote the user wrote, not `Quoted` |
| fenced or indented code | `Block::Pre` | the language tag is dropped |
| `---` | `Block::Rule` | |
| `[t](http…)`, `<https…>`, bare URL | `Inline::Link` | only http, https and mailto (`Href`) |
| a link with any other scheme | its source text | never a link |
| `![alt](cid-of-attached)` | `Inline::Image{content_id}` | only images attached in this draft |
| `![alt](http…)` | source text | **never fetched** (ADR 0003's privacy rule, research R6) |
| table, `~~x~~`, task list, footnote, raw HTML | source text in a `Paragraph` | the Document has no such node; the text part carries the real Markdown anyway |

**Promises**:
- It is total: every input yields a Document and it never panics. It is
  fuzzed.
- `render(to_document(md)).1` passes `outgoing::harden` unchanged (the
  existing backstop test).
- For any Markdown whose constructs are all in the table above,
  `render(to_document(md)).1` equals the HTML the GTK composer produces for
  the same content (SC-006). The test builds both.
- `to_document(md).is_plain_text()` exactly when `md` uses no construct beyond
  paragraphs and breaks. That decides text-only sending (FR-021).

## `from_document`: reopening a GTK draft in the terminal

It is the inverse of the table above. `Quoted` blocks are *not* serialised;
they are returned separately and shown read-only (research R6).

**Promise**: `to_document(from_document(d)) == d` for every `d` without
`Quoted`. This is property-tested over generated Documents.

## `from_html`: reading

- Input is **always** the reader sanitiser's output. It is never raw mail.
- It keeps: headings h1–h6, emphasis, strong, strikethrough, code, pre,
  lists (with start numbers), blockquotes, tables (as GFM tables), rules,
  links (the target is kept even when the label differs), and `<details>`
  folds (as a marker the terminal reader turns into a `Fold` block).
- Images become `![alt](postio-image:<identity>)`. The terminal renderer draws
  that as a placeholder and never treats it as a URL.
- Anything else becomes its text content.

**Promises** (corpus-wide test, SC-005), for every message in
`crates/postio-model/tests/corpus/`:
- The output, once rendered, contains no `<tag>`, no script text, no
  `javascript:`, and no remote URL in an image position.
- After `postio_ui::terminal::sanitize`, no character in U+0000–U+001F
  (except `\n` and `\t`), U+007F–U+009F, or U+202A–U+202E / U+2066–U+2069
  remains from message content.

## Plain-text messages

These are **not** passed through `from_html` or parsed as Markdown. They are
shown verbatim, with `>` runs folded by `postio_body::quote`, so a line
beginning `#` is shown as written (US2 scenario 3).
