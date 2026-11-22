# Data model: Compose formatting

There is no storage change. A draft stores the HTML `Document::to_html`
generates (and the terminal's `body_markdown` when the terminal wrote it
last), so a wider document needs no migration (FR-003).

## `postio_body::Document`, after this spec

Changes are marked **new** or **changed**. Everything else is today's
type.

```text
Block
  Paragraph(Vec<Inline>)
  Heading { level: HeadingLevel, inlines }            changed: six levels
  List { kind: ListKind, items: Vec<ListItem> }       changed
  Quote(Vec<Block>)
  Quoted(Quoted)                                      unchanged (ADR 0033)
  Code { language: Option<Language>, text: String }   changed: was Pre(String)
  Rule
  Table(Table)                                        new

Inline
  Text(String)
  Strong(Vec<Inline>)
  Emphasis(Vec<Inline>)
  Strikethrough(Vec<Inline>)                          new
  Code(String)
  Link { href: Href, inlines }
  Image { content_id: ContentId, alt }
  RemoteImage { href: Href, alt }                     new: a chip (spec D7)
  Break
```

### `HeadingLevel` — changed

`One` to `Six`. `from_digit(1..=6)` maps one to one, and nothing narrows.
`digit()` gives `1`–`6`. #1485 may land first with three levels; this
widens it.

### `ListKind` — new

```text
ListKind
  Bullet
  Ordered { start: u32 }     start defaults to 1; 0 is allowed
```

It replaces `ordered: bool`, so a bullet list cannot carry a start number.

### `ListItem` — new

```text
ListItem
  task: Option<Task>         None: an ordinary item
  blocks: Vec<Block>
Task
  Open
  Done
```

GFM puts a task box on an item, not on a list, so a list may mix tasks
and ordinary items. Ordered lists may have tasks too. "Task list" in the UI
is a bullet list whose items all have a `task`; `task_list` toggles that
for the list at the caret.

### `Language` — new

A code block's info word. It is built only by `Language::parse`, which
accepts 1 to 32 characters, each an ASCII letter, a digit or one of
`+ # . _ -`. Anything else is refused, and the fence's text after the
first word is dropped. The same pattern as `Href` and `ContentId`: the
type cannot hold what the serialiser would have to escape.

### `Table` — new

```text
Table
  alignments: Vec<Alignment>          one per column; len >= 1
  header: Vec<Vec<Inline>>            one cell per column
  rows: Vec<Vec<Vec<Inline>>>         every row has alignments.len() cells
Alignment
  None | Left | Center | Right
```

- **Invariant: rectangular.** Only `Table::new` builds one, and it pads a
  short row with empty cells and truncates a long one. GFM does the same,
  and the truncated cells are reported through `narrow::Lost`.
- **Cells hold inlines only**, so no block nests in a cell. `Break` is the
  line break inside a cell, as GFM's `<br>` is.
- **No spans.** `parse` narrows a `<table>` holding `colspan`, `rowspan`, or
  block content in a cell to paragraphs, as it does today (FR-002).
- **Allowed in** a quote's blocks and a list item's blocks, as CommonMark
  allows. **Never in** a table cell, because the type cannot express it.

### `Inline::Strikethrough` — new

The same shape as `Strong`. It nests with the others in any order.

### `Inline::RemoteImage` — new

An image the person named by address but has not attached. `href` is an
`Href` whose scheme is `http` or `https` (a `mailto:` image is not an
image, so it stays text).
- Only `markdown::to_document` creates one, and `parse` reads back the
  marker this spec emits.
- `parse` never makes one from a remote `<img>`, which still narrows away.
- It is never emitted as `<img>`. The type is what keeps "Postio cannot
  send a tracking pixel" true.
- A successful fetch replaces it with `Image { content_id, alt }` at the
  same place, as one undo step.

## `markdown::SourceMap` — new

```text
from_document_mapped(&Document) -> (String, SourceMap)
to_document_mapped(&str) -> (Document, SourceMap)
SourceMap
  to_source(DocPosition) -> usize       a byte offset in the Markdown
  to_document(usize) -> DocPosition     the nearest text position (spec "The caret on each switch")
DocPosition { path: Vec<usize>, offset: usize }   block index path, then a char offset in that block's text
```

It is built from `pulldown-cmark`'s `into_offset_iter` on the way in, and
by the writer recording each text run's offset on the way out. It is used
only to place the caret, and is not stored.

The carried quote's line in the source view is one private-use character
(`U+E001`), with the quote's index held beside the text. `to_document`
reads that line as the quote. The view strips `U+E001` from typed and
pasted text, so it cannot be forged.

## The HTML subset, after this spec

`to_html` emits, and `parse` reads back, these forms. They are the normal
form, so `to_html(parse(h)) == h` holds for each:

| Shape | HTML |
|---|---|
| `Heading` level n | `<hN>…</hN>`, n in 1..=6 |
| `Strikethrough` | `<del>…</del>` (parse also reads `<s>`, `<strike>`) |
| `List`, `Bullet` | `<ul>…</ul>` |
| `List`, `Ordered { start: 1 }` | `<ol>…</ol>` |
| `List`, `Ordered { start: n }` | `<ol start="n">…</ol>` |
| A list whose every item has a task | `<ul data-postio-task="list" style="list-style-type:none">` |
| An item with `task: Open` | `<li data-postio-task="open">☐ …</li>` |
| An item with `task: Done` | `<li data-postio-task="done">☑ …</li>` |
| `Code { language: None }` | `<pre>…</pre>` |
| `Code { language: Some(l) }` | `<pre data-postio-lang="l">…</pre>` |
| `RemoteImage` | `<a href="…" data-postio-remote-image>alt</a>` (a recipient sees a link) |
| `Table` | `<table border="1" cellpadding="6" cellspacing="0"><thead><tr><th align?>…</th></tr></thead><tbody><tr><td align?>…</td></tr></tbody></table>` |

- **`parse` reads the markers.** It reads `data-postio-task` and removes
  the generated `☐ ` or `☑ ` from the start of the item. Without the marker,
  a GitHub-style `<li><input type="checkbox" [checked]>` is read as a task
  as well; that is pasted HTML, before any sanitiser. Any other leading
  glyph is text.
- **`parse` ignores** the table's `border`, `cellpadding` and `cellspacing`,
  and a list's `style`. They are generated (D5) and carry nothing the
  document holds.
- **`editor_html`** emits the same, with the editing scheme for images, as
  today. The editing shell's stylesheet draws the task box as a clickable
  control over the glyph, with `role="checkbox"` and `aria-checked`. The
  wire form keeps the glyph.

## `postio_ui::editor::markdown::Sequence`, after this spec

```text
Sequence
  marker: Marker
  command: &'static str           a formatting command id (FR-068)
  trigger: Trigger
Marker
  Literal(&'static str)           today's form
  Ordered                         digits{1,9} then '.' or ')'
  Fence                           ``` or ~~~, then an optional Language
  DelimiterRow                    | :?-+:? | … with the previous block a pipe row of equal width
Trigger
  LinePrefix | Wrapping | LineComplete | WordComplete
```

## New registry entries

Every entry is in `Context::Composer`, with `Recovery::None` (the editor's
own undo takes an edit back, as for `bold`) and `MAIL`, unless noted.

**Phase** says when the row is registered (spec D8):
- **Now**: answered in the terminal and in the Markdown source view. In
  the rich view it gives its reason (FR-029).
- **Held**: registered with the native composer that answers it, so that
  no row exists that nothing runs.

| Id | Title | Default | Alternate | Phase | Notes |
|---|---|---|---|---|---|
| `heading_1`…`heading_6` | Heading 1…6 | `mod+alt+1`…`6` | `alt+1`…`6` | Now | D3; reshapes #1485's command |
| `normal_text` | Normal text | `mod+alt+0` | `alt+0` | Now | |
| `strikethrough` | Strikethrough | `mod+shift+s` | `alt+asciitilde` | Now | |
| `inline_code` | Inline code | `mod+e` | `alt+grave` | Now | |
| `code_block` | Code block | `mod+alt+c` | `alt+C` | Now | |
| `code_language` | Code language… | `mod+alt+C` | — | Held | `GRAPHICAL_MAIL` |
| `task_list` | Task list | `mod+shift+6` | `alt+t` | Now | |
| `toggle_task` | Done or not done | `mod+alt+x` | `alt+X` | Now | |
| `indent` | Indent | `mod+bracketright` | `alt+greater` | Now | in the source view and the terminal, shifts the item one level; Tab in a rich list item runs it (held) |
| `outdent` | Outdent | `mod+bracketleft` | `alt+less` | Now | the reverse; Shift+Tab in a rich list item runs it (held) |
| `divider` | Divider | `mod+alt+minus` | `alt+minus` | Now | |
| `line_break` | Line break | `shift+Return` | — | Held | `GRAPHICAL_MAIL` |
| `table` | Table… | `mod+shift+t` | `alt+bar` | Now | now: inserts a two-by-two template; held: inside a table, opens the table picker |
| `clear_formatting` | Clear formatting | `mod+backslash` | `alt+backslash` | Now | |
| `formatting` | Formatting… | `mod+alt+f` | `alt+F` | Held | opens the formatting picker |
| `paste_markdown` | Paste as Markdown | `mod+alt+v` | `alt+V` | Held | |
| `toggle_markdown_source` | Edit as Markdown or rich text | `mod+alt+s` | — | Now | `GRAPHICAL_MAIL` |
| `attach_remote_image` | Download and attach image | `mod+alt+i` | `alt+I` | Now | the one network gesture (FR-031) |
| `table_row_below`, `table_row_above` | Add row below / above | `1`, `2` | — | Held | `Context::TablePicker`, `GRAPHICAL_MAIL` |
| `table_column_right`, `table_column_left` | Add column right / left | `3`, `4` | — | Held | as above |
| `table_delete_row`, `table_delete_column` | Delete row / column | `5`, `6` | — | Held | as above |
| `table_align_left`, `table_align_center`, `table_align_right` | Align column left / centre / right | `l`, `c`, `r` | — | Held | as above; repeating clears it |
| `table_delete` | Delete table | `Delete` | — | Held | as above |
| `picker_choose_0`, `picker_choose_5`…`9` | Choose option N | `0`, `5`…`9` | — | Held | `Context::Picker`; widens the frame (R9) |

`menu.rs` puts every `Composer` entry above in `Format`. The table picker's
entries and the picker's choose keys appear in no menu, like
`forward_what_*`.
