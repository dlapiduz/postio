# Contract: typed sequences

`postio_ui::editor::markdown::SEQUENCES` is the table. The GTK and Mac
surfaces each have a test that drives every row through the real surface
and finds the construct in the document. The same table is the contract
ADR 0039's native surface must meet.

## Rules every row obeys

1. **FR-068.** `command` is a formatting command id in
   `Context::Composer`. A test holds the set.
2. **Longest first** within a trigger. A test holds that no row is
   shadowed by an earlier one.
3. **One undo** returns the literal characters, unconverted (FR-070). The
   surface posts the literal state before the conversion, as `editor.js`
   does today.
4. **Never inside** `<code>`, `<pre>`, or an element carrying
   `data-postio-quoted` (FR-010).
5. **Escapes.** A marker preceded by `\` is literal. The backslash is
   consumed, and the character never opens or closes a sequence afterwards.
6. **Flanking.** `_` opens only after a non-alphanumeric character (or the
   start), and closes only before one (or the end). `*` and `~~` may be
   intraword.

## The rows

| Trigger | Marker | Command | Effect |
|---|---|---|---|
| Wrapping | `***` | `bold` + `italic` | both, over the content |
| Wrapping | `___` | `bold` + `italic` | as above, with flanking |
| Wrapping | `**` | `bold` | |
| Wrapping | `__` | `bold` | flanking |
| Wrapping | `~~` | `strikethrough` | |
| Wrapping | `*` | `italic` | |
| Wrapping | `_` | `italic` | flanking |
| Wrapping | a backtick run of length n | `inline_code` | closes on an equal run; the content is not marked up |
| Wrapping | `[text](url)`, closed by `)` | `insert_link` | when `Href::parse(url)` accepts; otherwise literal |
| Wrapping | `<url>`, closed by `>` | `insert_link` | as above |
| WordComplete | a GFM autolink (research R2) | `insert_link` | on the space or punctuation after it |
| LinePrefix | `######` … `#` | `heading_6` … `heading_1` | |
| LinePrefix | `- [ ]`, `* [ ]`, `+ [ ]` | `task_list` | an open task item |
| LinePrefix | `- [x]`, `* [x]`, `+ [x]` (`X` too) | `task_list` + `toggle_task` | a done task item |
| LinePrefix | `[ ]`, `[x]` at the start of a list item | `task_list` | makes that item a task |
| LinePrefix | `-`, `*`, `+` | `bullet_list` | in a list item: changes the list's kind at that depth |
| LinePrefix | Ordered (`7.`, `7)`) | `numbered_list` | starts at 7 |
| LinePrefix | `>` | `quote_block`; `indent` when already in a quote | |
| LineComplete | `===` | `heading_1` | the block above, or the line above in this paragraph |
| LineComplete | `---` on the line under one paragraph line | `heading_2` | setext |
| LineComplete | `---`, `***`, `___` alone in a block | `divider` | |
| LineComplete | Fence | `code_block` (+ `code_language`) | opens an empty code block, caret inside |
| LineComplete | DelimiterRow | `table` | the pipe row above becomes the header |

## Not typed live

Some constructs are read only by `to_document`, for Paste as Markdown and
the terminal: reference links and definitions, indented code, entities,
and hard breaks written as two trailing spaces. A typed `\` then Enter is a
line break in every surface, and two trailing spaces then Enter is one in
the rich surfaces.
