# Feature Specification: Compose formatting — everything Markdown can say

**Feature Branch**: `feature/compose-formatting`

**Created**: 2026-11-22

**Status**: Draft. The maintainer answered Q1–Q4 on 2026-11-22
(Clarifications); the spec is ready for review of the folded answers

**Input**: The maintainer, 2026-11-22: "I want to be able to do everything
that I can do on a Markdown file", and "I think we need to add more
formatting not less, maybe we need a spec for the formatting window". The
same day, on #1485, they decided to add a heading command: "The direction is
more compose formatting, not less."

## Context

The composer is rich text over Postio's own document (`postio_body::Document`,
ADR 0004). Markdown is a way to *type* formatting, not the draft's format
(ADR 0003, spec 002 FR-067 to FR-072). This spec keeps both rules. It widens
what the document can hold, and adds a command for each new construct, until
the composer can author everything CommonMark and GitHub-Flavored Markdown
(GFM) can express.

What exists on `main` (0f160a97), and what this spec adds:

| Construct | Document | Command | Typed sequence | Terminal composer |
|---|---|---|---|---|
| Bold, italic | `Strong`, `Emphasis` | `bold`, `italic` | `**x**`, `*x*`, `_x_` | wraps the markers |
| Bulleted, numbered list, nested | `List { ordered, items }` | `bullet_list`, `numbered_list` | `- `, `* `, `1. ` | prefixes |
| Quote | `Quote` | `quote_block` (toggles) | `> ` | prefix |
| Link | `Link { href }` (http, https, mailto) | `insert_link` | — | wraps |
| Inline image | `Image { content_id }` (`cid:` only) | `insert_image`, paste, drop | — | `![alt](cid:…)` |
| Heading | `Heading`, **levels 1–3**; `h4`–`h6` narrow to 3 | **none**: #1485, in flight | **none**: a test refuses `#` until the command exists | `#` read on send |
| Inline code | `Code` | **none** | **none** | read on send |
| Code block | `Pre(String)`, **no language** | **none** | **none** | fence read on send |
| Divider | `Rule` | **none** | **none** | `---` read on send |
| Hard line break | `Break` | none (Shift+Enter in the surface) | **none** | `\` + newline |
| Strikethrough | **none** | **none** | **none** | stays literal `~~x~~` |
| Task list | **none** | **none** | **none** | stays literal |
| Table | **none** (only inside a carried reply quote) | **none** | **none** | stays literal pipes |
| Ordered list start | **none** (always 1) | — | — | lost |
| Escapes | `\*` read by the terminal parser | — | **none** | read on send |

Every composer edits the same document: GTK's (`postio-widgets`' composer,
a WebKit `contenteditable` running `editor.js`), the Mac's (a `WKWebView`
running the same script from `postio_ui::compose`), and the terminal's
(Markdown text through `postio_body::markdown::to_document`). ADR 0039 plans
to replace the WebKit surface with a native text view over an edit algebra in
`postio-body`; it is accepted but not built.

### What this spec inherits

- **ADR 0003** (rich text, the restricted subset, the privacy rules).
  **ADR 0004** (one closed document type; outgoing HTML is generated, never
  passed through; `parse` is total and narrows). **ADR 0033** (a reply's
  quote is the original as the reader showed it). **ADR 0039** (the
  native-surface plan).
- **Spec 002** (`docs/archive/specs/002-compose-editor/`): FR-037 (the
  plain-text alternative stays readable) and FR-067 to FR-072 (Markdown as
  input). **FR-068 stands unchanged**: a sequence may reach only a
  formatting command. This spec adds the commands, so the sequences follow
  them.
- **Spec 007** (Focus): the composer's layout ("The composer" in
  `screens.md`), FR-092 (one picker pattern, one dialog pattern, one icon
  button, one keycap), and the interaction rules.
- **#1485**: the heading command. This spec does not build it; it states
  the level set and keys the command must end with (D3).
- **The constitution**: II (the keyboard is a system), IV (test-first),
  V (performance), VI (privacy) and VII (boundaries).

### Decisions this spec takes

These are design calls an agent can make. Each is reviewable; the three
that only the maintainer can make are under Clarifications.

**D1. Markdown is a way in, and a view, but never the stored format.** A
draft is a `Document`, and ADR 0003 stands. Markdown reaches it two ways:
- typed sequences;
- the Markdown source view (Q1 = B, "The Markdown source view"), which
  edits the same draft as Markdown text and reads it back on return.

**D2. Every construct has a command, a key, a typed sequence, a picker row
and a rendering.** That is FR-068 read forwards: rather than forbid a
sequence, add the command it needs.

**D3. Headings are six commands, not one.** `heading_1` to `heading_6`
and `normal_text`, on Google Docs' `mod+alt+<digit>` (Gmail has no
headings; Postio's list keys already follow Docs' `mod+shift+7/8/9`).
Every level is one chord, with no cycling. If #1485 lands as one `heading`
command, this spec's first task reshapes it into these seven. The ids are
a file format (`[keys]`), so the reshape has to land before a release
ships the old id.

**D4. The formatting window is a picker.** It is a popover anchored at the
caret, in spec 007's one picker pattern. It is not a window, not a second
command bar and not a panel (Q2 = A). See "The formatting picker". It is
built on ADR 0039's native composer (D8).

**D5. Generated presentation is not styling.** ADR 0004 forbids styling the
*user* controls. The serialiser may emit a fixed set of presentational
attributes on four elements, so that a recipient's client draws the
construct. `parse` ignores those attributes on the way back in:

- `border`, `cellpadding` and `cellspacing` on `<table>`;
- `align` on `<th>` and `<td>`;
- `list-style-type:none` on a task list's `<ul>`;
- the `data-postio-*` markers that carry a code block's language and a
  task's state through the draft's HTML.

None of them can be set from a document, and no document holds a colour,
a font or a class.

**D6. The plain-text alternative keeps a marker when dropping it would
change the meaning.** The alternative drops the markers for bold and italic
(as today). It keeps the markers for strikethrough, task state, headings,
code fences and tables, because without them the words read differently:
"~~Tuesday~~ Wednesday" is not "Tuesday Wednesday". The table is under
"Every construct".

**D7. A remote image is fetched once, on a press, or never.** A remote
image (`![alt](https://…)`) typed or pasted as Markdown becomes a "Download
and attach" chip (Q3 = B):
- pressing it fetches the image once and attaches it as a `cid:` inline
  image;
- until it is pressed, nothing is fetched;
- if it is never pressed, it is sent as a link.

See "Remote images: the Download and attach chip".

**D8. Build the model, the wire, Markdown in and out, the terminal, the
registry, the source view and the chip now. Hold the rich editing surface
for ADR 0039's native composer (Q4 = C).**

| Now | Held for ADR 0039 |
|---|---|
| The document, `parse`, `to_html`, the text alternatives, `harden` | Typed sequences in the rich surface (US1, US2) |
| `markdown::to_document`, `from_document`, GFM autolinks | Drawing the new constructs in the rich surface |
| The terminal composer, all of it | The formatting picker, the table picker and the code language picker (US4, US5) |
| The registry rows for what can be answered now (`data-model.md`) | The new toolbar buttons and their reflection |
| The Markdown source view in GTK and on the Mac (US7) | The structural edit ops (ADR 0039's `apply`) |
| The chip as a row under the editor, the fetch, and the send fallback (US8) | Paste as Markdown, the inline chip, `line_break` |

**The source view is the interim editing surface.** Until the native
composer exists, every construct is authored in GTK and on the Mac by
switching to Markdown, just as the terminal already authors them. The
reasons for this choice:
- **It is not throwaway work.** The source view is a plain text widget
  over `from_document` and `to_document`, which are built now anyway.
  ADR 0039's composer keeps the same toggle, and none of it is WebKit.
- **It delivers the maintainer's sentence now.** "Everything I can do on a
  Markdown file" holds on the day this lands, without a line of
  `editor.js` written to be thrown away.
- **The rich view does not lose a construct while it waits.** It shows
  what the source view made through `editor_html`, drawn by WebKit's
  defaults, and `parse` reads it back (FR-002).

**In the rich view, a new formatting command** is answered with its reason
and the way forward ("Strikethrough is in the Markdown view for now ·
ctrl+alt+s"). It is never silently dead. Bold, italic, the lists, quote,
link and image keep working as they do today.

**D9. Vocabulary.** The UI says "Divider" (not "horizontal rule" or
"thematic break"), "Task list" and "Done" (not "checklist" or "checked"),
"Code block" and "Inline code", and "Strikethrough". These are new words
for the app, recorded here as the vocabulary decision the ux-architect
skill asks to be named.

## Clarifications

### Session 2026-11-22 (the maintainer)

- **Q1.** Does "everything I can do on a Markdown file" include editing the
  Markdown itself?
  → **B: a Markdown source toggle.** The draft can be switched to edit as
  Markdown text and back. The stored format stays the `Document`, so
  ADR 0003 stands. See "The Markdown source view", US7 and FR-023 to
  FR-029.
- **Q2.** What should the formatting window be?
  → **A: the picker anchored at the caret.** See "The formatting picker".
- **Q3.** What happens to a typed `![alt](https://…)`?
  → **B: a "Download and attach" chip.** It fetches the image once on a
  press and attaches it as a `cid:` inline image. Nothing is fetched until
  then. See "Remote images", US8 and FR-030 to FR-035.
- **Q4.** Build the surface now, or after ADR 0039?
  → **C: land the model, the wire, the Markdown input, the terminal and the
  registry now; hold the editing surface** (the visual formatting in the
  composer, and the formatting window's UI) for ADR 0039's native composer.
  This spec puts the source view and the chip's row form in the now
  phase (D8).

## Every construct

The registry rows are new unless marked *(exists)* or *(#1485)*. Keys are
`mod` (Control, or Command on a Mac). The second key in each cell is the
alternate a legacy terminal can deliver (`postio_ui::terminal::legacy_deliverable`).
"Not the terminal" means the terminal composer has no surface for it,
because there the Markdown is typed as text.

### Inline

| Construct | Command · keys | Typed | In the composer | HTML sent | Plain text sent |
|---|---|---|---|---|---|
| Bold | `bold` · `mod+b` *(exists)* | `**x**`, `__x__` (new) | Bold weight | `<strong>` | `x` |
| Italic | `italic` · `mod+i`, `alt+i` *(exists)* | `*x*`, `_x_` | Italic | `<em>` | `x` |
| Bold italic | both | `***x***`, `___x___` | Both | `<strong><em>` | `x` |
| Strikethrough | `strikethrough` · `mod+shift+s`, `alt+asciitilde` | `~~x~~` | A line through, muted ink | `<del>` | `~~x~~` (D6) |
| Inline code | `inline_code` · `mod+e`, `alt+grave` | `` `x` ``, and longer backtick runs | Adwaita Mono on the code ground | `<code>` | `x`, backticks kept only when the run holds a space at its edge |
| Link | `insert_link` · `mod+shift+k`, `alt+k` *(exists)* | `[text](url)`; `<url>`; a bare `https://…`, `www.…` or address followed by a space or punctuation (GFM's autolinks) | Accent, underlined | `<a href>` (http, https, mailto only) | `text <url>`; the bare address when the text is the address |
| Image | `insert_image` · `mod+shift+g`, `alt+g` *(exists)* | `![alt](cid:…)`, naming a part of this draft | Inline, at its size, capped to the column | `<img src="cid:…" alt>` in `multipart/related` | `[image: alt]` |
| Remote image | `attach_remote_image` · `mod+alt+i`, `alt+I` | `![alt](https://…)` (or `http://`) | A "Download and attach" chip (D7) | until pressed: `<a href>alt</a>`; once pressed: as Image | until pressed: `alt <url>` |
| Hard line break | `line_break` · `shift+Return` (not the terminal) | two spaces then Enter, or `\` then Enter | A new line in the same paragraph | `<br>` | a newline |
| Escape | none: an escape is the way *not* to run a command | `\` before any ASCII punctuation | The punctuation, literally; it never pairs with a later marker | the character | the character |
| Clear formatting | `clear_formatting` · `mod+backslash`, `alt+backslash` | — | Removes inline marks from the selection | — | — |

### Blocks

| Construct | Command · keys | Typed | In the composer | HTML sent | Plain text sent |
|---|---|---|---|---|---|
| Heading 1–6 (ATX) | `heading_1`…`heading_6` · `mod+alt+1`…`6`, `alt+1`…`6` *(#1485, reshaped by D3)* | `# ` … `###### ` at a block's start | The reader's heading scale, six steps | `<h1>`…`<h6>` | H1: the text over `=`s of its width; H2: over `-`s; H3–H6: `### text` |
| Heading (setext) | the same commands | `===` or `---` on the line under one paragraph line (after Shift+Enter), then Enter; `===` alone in the block after a one-line paragraph, then Enter | As above | As above | As above |
| Normal text | `normal_text` · `mod+alt+0`, `alt+0` | — | Body text | `<p>` | the text |
| Quote | `quote_block` · `mod+shift+9`, `alt+9` *(exists)* | `> ` | A rule at the left, muted ink | `<blockquote>` | `> ` |
| Nested quote | `indent` · `mod+bracketright`, `alt+greater`; `outdent` · `mod+bracketleft`, `alt+less` | `> ` at the start of a block already in a quote | One more rule per level | nested `<blockquote>` | `>> `, `>>> ` (RFC 3676) |
| Bulleted list | `bullet_list` · `mod+shift+8`, `alt+8` *(exists)* | `- `, `* `, `+ ` | Bullets | `<ul>` | `- ` |
| Numbered list | `numbered_list` · `mod+shift+7`, `alt+7` *(exists)* | `1. ` or `1) `, with any number up to nine digits, which becomes the first number | Numbers from the start | `<ol>`, with `start` when not 1 | `N. ` from the start |
| Nested list | `indent`, `outdent`; Tab and Shift+Tab inside a list item | a list sequence at the start of an item changes that list's kind at its depth | Indented one step | nested `<ul>`/`<ol>` in `<li>` | four spaces per level |
| Task list | `task_list` · `mod+shift+6`, `alt+t` | `[ ] ` or `[x] ` at the start of a list item; `- [ ] ` at a block's start | A box before each item; a done item's text muted, not struck | `<ul style="list-style-type:none">` and items starting with `☐` or `☑` (D5) | `- [ ] ` / `- [x] ` |
| Done / not done | `toggle_task` · `mod+alt+x`, `alt+X`; a click on the box | `x` typed inside the brackets | The box fills | `☑` / `☐` | `[x]` / `[ ]` |
| Code block | `code_block` · `mod+alt+c`, `alt+C` | ```` ``` ```` or `~~~`, an optional language, then Enter | Adwaita Mono on the code ground, the language as a muted label at its top right; no sequence fires inside | `<pre data-postio-lang="…">`; the language is kept for the round trip only | fenced with ```` ``` ```` and the language, verbatim, never reflowed |
| Code language | `code_language` · `mod+alt+C` (not the terminal) | the word after the opening fence | The label | — | the fence's info word |
| Divider | `divider` · `mod+alt+minus`, `alt+minus` | `---`, `***` or `___` alone in a block, then Enter | A hairline across the column | `<hr>` | `----` |
| Table | `table` · `mod+shift+t`, `alt+bar` | a pipe row, Enter, then a delimiter row (`\|---\|:-:\|`) with the same number of cells, then Enter | A grid of hairlines, the header bold, each column aligned | `<table border="1" cellpadding="6" cellspacing="0">`, `<th align>`, `<td align>` (D5) | A GFM pipe table, padded to column widths, with its alignment row |

Pasted Markdown and the terminal composer also read three CommonMark
constructs that have no typed sequence, because no person types them live:

- reference links (`[text][ref]` with a `[ref]: url` line), which are
  resolved to links;
- indented code blocks (four spaces), which become code blocks;
- entity references (`&copy;`), which are decoded.

Raw HTML stays the literal text, as today. Footnotes, maths and front matter
are not CommonMark or GFM and are out of scope.

## The formatting picker

The surface the maintainer called the formatting window (D4, Q2 = A).
**Held for ADR 0039** (D8): it is drawn on the native composer.

- **Opening it.** `formatting` ("Formatting…", `mod+alt+f`, `alt+F`)
  anchors it at the caret's line. The toolbar's last button, an icon menu
  button whose tooltip is "Formatting · ctrl+alt+f", anchors it below
  itself. The command bar lists every formatting command too, as it lists
  every command.
- **What it names.** Like every picker, it says what it acts on: "Selection
  · 4 words", "Paragraph", "List item", "Table · row 2, column 3".
- **Rows.** It has a filter field, then four groups:
  - Text: Bold, Italic, Strikethrough, Inline code, Link…, Clear
    formatting.
  - Paragraph: Normal text and Heading 1–6, as one row of seven compact
    cells.
  - Lists: Bulleted, Numbered, Task list, Indent, Outdent.
  - Blocks: Quote, Code block, Table…, Divider, Image…, Line break.

  Each row shows its icon, its title, its typed sequence in muted Adwaita
  Mono (`~~text~~`, `## `), and its keycap at the right (`hints::short`).
  A row that is in force at the caret carries a check. The keys and the
  Markdown are taught inside the control.
- **Keys.** Typing filters ("str" leaves Strikethrough). `Up` and `Down`
  move. `Return` runs the row and closes. `Esc` closes and changes nothing,
  and the caret is where it was. With the filter empty, `1` to `6` choose
  the heading levels and `0` chooses Normal text: these are the picker's
  presets, as number keys are in every picker. A formatting command's own
  chord runs it and closes.
- **The toolbar.** The row under the fields gains four buttons after the
  existing ones, all in the shared 26 px ghost:
  - Strikethrough, after Italic;
  - Task list, after Numbered;
  - Inline code, after Quote;
  - Formatting, at the end.

  Headings, code blocks, tables, dividers and line breaks are reached
  through the picker, their keys and their Markdown, never by a toolbar
  button. When the column is too narrow for the row, buttons leave from the
  right, before Formatting. Formatting always stays, so nothing is out of
  reach.
- **The table picker.** `table` inside a table opens the same picker
  pattern, anchored at the cell, with:
  - `1` row below, `2` row above, `3` column right, `4` column left;
  - `5` delete row, `6` delete column;
  - `l`, `c` and `r` to align the column (pressing the alignment it already
    has clears it);
  - `Delete` to remove the table.

  These are registry commands in a "Table picker" context, as
  `forward_what_1`–`3` are in the forward chooser's. Tab in a table's last
  cell adds a row, as every table editor does.
- **The code language picker.** `code_language` opens the picker anchored
  at the code block. The last four languages used are its `1`–`4` presets;
  typing names any other. A language is a word of up to 32 letters, digits
  and `+#._-`.

| State | What the formatting picker shows |
|---|---|
| Empty filter result | "No formatting called "xyz"" and "Esc to close": never a blank list |
| A command that cannot apply here | The row is shown, unavailable, with the reason as its subtitle: "Tables don't go inside tables", "The quoted message is kept as it was sent" |
| Loading, partial, offline, failing | Cannot happen: everything is local and nothing it does reaches the network |
| Narrow (1024 × 768) | It fits the 615 px column. The Paragraph row wraps to two lines before any row is cut |

## The Markdown source view

Q1 = B. **Built now**, and the interim editing surface until ADR 0039
(D8).

- **The command.** `toggle_markdown_source`, titled "Edit as Markdown or
  rich text", on `mod+alt+s`. It is GTK and Mac only (`GRAPHICAL_MAIL`):
  the terminal composer is always Markdown. A toggle button sits at the
  toolbar's right end, before Formatting once that exists. Its tooltip
  carries the key, and it shows pressed while the view is Markdown.
- **What it shows.** The draft as `from_document` writes it, in a plain
  text view set in Adwaita Mono on the editor's surface. A one-line strip
  at the top says "Editing as Markdown · ctrl+alt+s for rich text", so the
  mode cannot be missed. The subtitle still says what will be sent
  ("Formatted · 58 words").
- **Formatting commands in the view** edit the Markdown: bold wraps the
  selection in `**`, Task list prefixes `- [ ] `. These are the same pure
  functions the terminal composer uses, moved to
  `postio_ui::editor::markdown_edit` so the two cannot differ. Every
  formatting command in `data-model.md`'s "now" rows works here.
- **Returning.** The text is read with `to_document`, and the result
  becomes the draft. The whole session in the view is one undo step in
  the draft's history. Inside the view, ordinary text undo works keystroke
  by keystroke.
- **The round trip is lossless (FR-024).** `to_document(from_document(d))
  == d` holds for every document in normal form: adjacent text merged, and
  nothing empty but a paragraph. That covers every construct in scope.
  Where CommonMark has no spelling for a shape, `from_document` writes a
  spelling `to_document` reads back:
  - `<br>` for a line break Markdown cannot place: in a heading, in a
    table cell, or at the start or end of a paragraph;
  - a line holding only `<br>` for an empty paragraph;
  - an empty `<!-- -->` line between two adjacent lists of the same kind,
    which is CommonMark's own idiom for splitting them;
  - `&#32;` for a space Markdown would strip at the start or end of a
    paragraph.

  `<br>` and `<!-- -->` are the only raw HTML `to_document` reads as
  structure. Any other HTML stays the literal text, as today. The reverse
  direction, Markdown → document → Markdown, keeps the meaning but not the
  spelling (`*` may come back as `-`). That is why the draft also keeps
  the person's own text (next point).
- **What Markdown cannot express.** In a document, only a carried reply
  quote or forwarded message (`Block::Quoted`, ADR 0033) has no Markdown,
  because it is the sender's HTML kept as it was sent. In the view it is
  one atomic, read-only line, drawn as the "▸ Quoted message · kept as
  sent" chip, at its place among the blocks:
  - text can be typed above and below it;
  - Backspace or Delete removes it whole, and undo brings it back;
  - it cannot be edited, split or copied;
  - the private character that marks it is stripped from pasted or typed
    text, so it cannot be forged.

  On return, it is restored at that line. Typed Markdown that would
  narrow is reported: a ragged table's extra cells, a link to a refused
  scheme, an image naming a part this draft does not carry. The report is
  one notice line under the editor ("2 table cells were dropped: their rows
  are longer than the header"), with Undo. Nothing narrows silently.
- **The person's own spelling survives.** While the view is open, and when
  a draft is saved from it, the draft's existing `body_markdown` holds the
  text as typed, beside the HTML. A draft last edited as Markdown reopens
  in the view with that text, in GTK, on the Mac and in the terminal. When
  the rich view next saves, it clears `body_markdown`, as it does today.
- **The caret on each switch.** It lands on the same character of the
  person's text.
  - **Rich → Markdown.** The caret's position in the document (a block
    path and an offset into the block's text) maps through
    `from_document`'s source map to the offset of that character in the
    Markdown. The selection maps both ends.
  - **Markdown → rich.** The offset maps through `to_document`'s source
    map to the nearest text character:
    - inside a marker (`**`, `## `, `- [ ] `, a fence line, a delimiter
      row), the caret goes to the start of the text that marker formats;
    - on the quote chip's line, it goes to the start of the block after
      the quote;
    - in text that narrowed away, it goes to the nearest surviving
      character before it.
  - A selection keeps both ends if both map, and otherwise collapses to
    the caret. The caret's line stays at the same height in the view.

| State | What the source view shows |
|---|---|
| Empty draft | An empty text view with the strip; the caret at the start |
| A reply | The person's text, then the quote chip |
| Offline, failing | The same: the view is local, and nothing in it waits |
| Narrow | The strip shortens to "Markdown · ctrl+alt+s"; the text wraps at the column |

## Remote images: the Download and attach chip

Q3 = B. **Built now** in its row form; the inline chip waits for ADR 0039
(D8).

- **What makes a chip.** A remote image (`![alt](https://…)` or `http://`,
  through `Href::parse`) becomes a remote image in the document, a chip.
  It can arrive from any Markdown the composer reads:
  - the source view;
  - the terminal;
  - Paste as Markdown, once that is built.

  HTML pasted into the rich view keeps narrowing a remote `<img>` away, as
  today. The parser that reads hostile mail does not mint chips.
- **Where it shows now.** The composer's attachment list holds one row per
  remote image, under a heading "Not attached yet":
  - it names the alt text and the host ("Chart · from example.com");
  - it has a "Download and attach" button with the key `mod+alt+i` in it;
  - it has a remove button, which turns the image into its link.

  The terminal draws the same row in its attachment list. Once ADR 0039's
  native composer exists, the chip is drawn inline at the image's place
  as well.
- **The press.** `attach_remote_image` acts on the chip at the caret, or
  on the first unattached chip in reading order, which its row names. It
  makes exactly one HTTP(S) GET to that address, without blocking the UI:
  - no cookies, no credentials, no `Referer`, and a generic `User-Agent`;
  - at most 3 redirects, never to another scheme, and only to http(s);
  - a 15 s limit.

  The row says "Downloading…" while it runs. On success, the bytes are
  attached through the path Insert image uses (`postio_session::attaching`).
  The chip becomes the image, and the document's remote image becomes
  `Image { content_id }`.
- **Failure states.** Each says why, keeps the chip, and offers the same
  key again. A second press is a second request, and nothing retries by
  itself.

  | Failure | The row says |
  |---|---|
  | Unreachable (DNS, connection, TLS, timeout, an HTTP error status) | "Couldn't download from example.com · {reason} · Try again ctrl+alt+i" |
  | Too large (over 10 MB, by `Content-Length` or as the bytes arrive, when the download stops) | "Larger than 10 MB · not attached · it will be sent as a link" |
  | Not an image (the bytes are not PNG, JPEG, GIF or WebP, whatever the server's header says; SVG is refused because it can carry script) | "example.com sent something that isn't a picture · it will be sent as a link" |
  | Offline | "You're offline · press again when you're online". Nothing is queued: a fetch never happens later on its own |

- **If it is never pressed**, the chip is sent as a link: HTML
  `<a href="…">alt</a>`, and plain text `alt <url>`. Postio fetches
  nothing. The recipient sees a link, which their client fetches only if
  they follow it. Before Send, the composer's warning line says "1 image
  isn't attached and will be sent as a link · ctrl+alt+i to attach". Send
  stays live and does not ask: a link is a safe outcome, and Postio prefers
  undo to confirmation. Pressing Send while a download is running sends a
  link for that image, and the warning line has already said so.
- **What is logged**: the draft id, the outcome and the byte count. The
  address is never logged: it is message content.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Type the inline constructs and get them (Priority: P1)

**Held for ADR 0039** (D8). It is built on the native composer. Until then, the
same constructs are written in the Markdown source view (US7).

A person writes `~~Tuesday~~ Wednesday works, run `make check` first, see
https://example.com/plan`. As they type, the strikethrough, the inline code
and the link appear formatted and the markers go. One `mod+z` after any
conversion puts back the literal characters, unconverted (spec 002 FR-070).
`snake_case_name` stays as typed, and so does `2 * 3 * 4`.

**Why this priority**: It is what "everything I can do on a Markdown file"
means at the keyboard, and it is the cheapest slice: it changes the document
by one inline kind and the sequence table by a few rows.

**Independent Test**: Through the GTK composer suite, type each sequence
into the body. Assert that the document holds the construct, that the sent
HTML and the plain text match the construct table, and that one undo
restores the characters. The sequence rules themselves are unit cases in
`postio_ui::editor::markdown`.

**Acceptance Scenarios**:

1. **Given** an empty body, **When** the person types `~~gone~~`, **Then**
   "gone" is struck through, no tilde remains, and the plain text sent
   reads `~~gone~~`.
2. **Given** `` `make check` `` typed, **Then** it is inline code, and
   `**` typed inside it stays two asterisks.
3. **Given** `https://example.com/plan` followed by a space, **Then** it is
   a link to that address. One undo leaves it as plain words.
4. **Given** `[plan](javascript:alert(1))` typed, **Then** it stays the
   literal text, and nothing in the sent HTML points at a script.
5. **Given** `snake_case_name` typed, **Then** nothing becomes italic:
   CommonMark's rule that `_` does not open or close inside a word.
6. **Given** `\*not italic\*` typed, **Then** the body reads `*not italic*`
   with no backslashes and no italics. Typing a later `*` does not pair
   with either asterisk.
7. **Given** a selection, **When** `mod+shift+s` is pressed, **Then** the
   selection is struck through and the toolbar's Strikethrough button shows
   it is in force.

---

### User Story 2 - Type the block constructs and get them (Priority: P1)

**Held for ADR 0039** (D8). It is built on the native composer. Until then, the
same constructs are written in the Markdown source view (US7).

A person writes a short plan: `## Plan` and a space makes a heading. `- [ ] `
starts a task list. Tab nests an item. `3. ` starts a numbered list at
three. ```` ```rust ```` and Enter opens a code block labelled rust. `> `
inside a quote nests it. `---` and Enter draws a divider. Every one is also
a command with a key, and every one undoes in one step.

**Why this priority**: Headings, task lists and code blocks are what people
mean by "a Markdown file". This story carries the most model change: six
heading levels, list starts, tasks and code languages.

**Independent Test**: As US1, block by block. The model changes are unit
cases in `postio-body` (`parse(to_html(d)) == d` for every new shape, and
`to_html(parse(h)) == h` for HTML already in the subset).

**Acceptance Scenarios**:

1. **Given** an empty block, **When** `#### ` is typed, **Then** the block
   is a level-4 heading, and the sent HTML has `<h4>`.
2. **Given** a one-line paragraph "Plan", **When** Enter, `===` and Enter
   are typed, **Then** "Plan" is a level-1 heading and the `===` block is
   gone.
3. **Given** `---` alone in a block, **When** Enter is typed, **Then** it
   is a divider, not a heading. Setext level 2 needs the `---` on the line
   under the text, after Shift+Enter.
4. **Given** `- [ ] buy milk` and a click on its box, **Then** the item is
   done. The HTML sent starts the item with `☑`, and the plain text with
   `- [x] `.
5. **Given** `7. ` typed, **Then** a numbered list starts at 7, and the
   HTML sent has `<ol start="7">`.
6. **Given** ```` ```rust ```` and Enter, **Then** a code block labelled
   "rust" opens. `**x**` typed inside it stays literal. The plain text sent
   is fenced with ```` ```rust ````.
7. **Given** the caret in a quote, **When** `> ` is typed at the start of a
   block, **Then** that block is quoted one level deeper, and the plain
   text sent marks it `>> `.
8. **Given** the caret in a list item, **When** Tab is pressed, **Then**
   the item nests under the one above. Shift+Tab undoes the nesting. Tab
   outside a list or quote still moves the keyboard out of the body.

---

### User Story 3 - A recipient sees what was written, in any client (Priority: P1)

**Now** (D8).

What the person formatted reaches every recipient readably. The HTML part
draws the constructs in clients that strip styles, and in Outlook's Word
engine. The plain-text part reads as well-formed Markdown, with the markers
kept wherever dropping them would change the meaning. Postio's own reader
shows a sent message as it was written.

**Why this priority**: Formatting that arrives broken is worse than none.
Mail HTML is not a web page: no `<input>`, no stylesheet, no class
reliably survives.

**Independent Test**: Golden cases in `postio-body` render one document
with every construct through `render()`. The HTML is pinned, and so is the
plain text. The same document read back by `markdown::to_document` must
equal it. The `harden` backstop must change nothing. A send through the
`MailBackend` mock carries both parts. Postio's reader draws the HTML part
with every construct visible (a `postio-render` snapshot).

**Acceptance Scenarios**:

1. **Given** a table with a right-aligned column, **When** it is sent,
   **Then** the HTML carries `border`, `cellpadding` and `align` attributes
   and no `style` or `class` on the table. The plain text is a padded pipe
   table whose alignment row says `--:`.
2. **Given** a task list, **When** it is sent, **Then** no `<input>`
   appears, and each item begins with `☐` or `☑`.
3. **Given** a code block with a 120-character line, **When** it is sent as
   `format=flowed`, **Then** the line is not wrapped or joined, and a line
   starting with `>` or `From ` inside it is space-stuffed.
4. **Given** a message using only bold, **Then** it is still sent as
   `html + text/plain` and the plain text has no asterisks (spec 002
   FR-072).
5. **Given** a message using no formatting at all, **Then** it is sent as
   `text/plain, format=flowed` alone, as today.

---

### User Story 4 - Find any formatting without knowing Markdown (Priority: P2)

**Held for ADR 0039** (D8). It is built on the native composer. Until then, the
same constructs are written in the Markdown source view (US7).

A person who has never typed Markdown presses `mod+alt+f`, or the toolbar's
Formatting button. They type "tab", press Return, and get a table. The
picker shows each command's key and its Markdown, so next time they type
it.

**Why this priority**: Typed sequences serve people who know Markdown.
The picker, the toolbar and the keys serve everyone, and they are what
Constitution II requires of every command.

**Independent Test**: The focus suite opens the composer, presses
`mod+alt+f`, types a filter, presses Return, and asserts the document and
the caret. A storyboard (`storyboards/compose/formatting-picker.toml`),
written from this story and red on the base, is filmed and judged by
`/ux-review`.

**Acceptance Scenarios**:

1. **Given** the caret in a paragraph, **When** `mod+alt+f` is pressed,
   **Then** the picker opens at the caret, names "Paragraph", and has the
   filter focused.
2. **Given** the picker with an empty filter, **When** `2` is pressed,
   **Then** the paragraph becomes Heading 2 and the picker closes.
3. **Given** the picker, **When** `Esc` is pressed, **Then** nothing
   changes and the caret is where it was.
4. **Given** the caret in bold text, **Then** Bold's row carries a check.
5. **Given** the composer at 1024 × 768, **Then** the toolbar's Formatting
   button is visible and every command is in the picker.
6. **Given** the key map (`?`) or the command bar, **Then** every command
   in the construct tables is listed with its key.

---

### User Story 5 - Write and edit a table (Priority: P2)

**Held for ADR 0039** (D8). It is built on the native composer. Until then, the
same constructs are written in the Markdown source view (US7).

A person types `| Item | Qty |`, Enter, and `|---|--:|`, Enter, and gets a
two-column table with Qty right-aligned. Tab walks the cells and adds a row
from the last one. `mod+shift+t` inside the table opens the table picker,
where they add a column and centre it.

**Why this priority**: Tables are the most-asked-for GFM construct and the
hardest to send well, so they get their own slice.

**Independent Test**: Table edits are pure functions on `Document`, unit
cases in `postio-body`: add or delete a row or a column, align, and pad
ragged rows. The surface is a GTK composer case plus a storyboard.

**Acceptance Scenarios**:

1. **Given** the two rows typed as above, **Then** the document holds a
   table with a header row, two columns, and alignments none and right.
2. **Given** the caret in the last cell, **When** Tab is pressed, **Then**
   a row is added and the caret is in its first cell.
3. **Given** the table picker, **When** `3` then `c` are pressed, **Then**
   a column is added to the right, the picker closes, and the next table
   picker's `c` centres it.
4. **Given** `mod+shift+t` outside a table, **Then** a two-by-two table
   with a header row is inserted and the caret is in its first cell.
5. **Given** a pasted HTML table with merged cells, **Then** it is narrowed
   to paragraphs, as today, and is not forced into a grid.

---

### User Story 6 - Paste Markdown, reopen a draft, use any composer (Priority: P2)

**Now**, except Paste as Markdown into the rich view (scenarios 1 and 2),
which is held for ADR 0039 (D8).

A person copies a README section and presses `mod+alt+v`. It arrives
formatted. An ordinary paste of the same text arrives as the text, because
text that resembles Markdown but was not converted is sent as it looks
(spec 002 FR-071). A draft saved with every construct reopens the same in
the GTK composer, on the Mac, and in the terminal composer, which reads and
writes the same constructs as Markdown text.

**Why this priority**: Without it, "everything a Markdown file can do"
holds only for what is typed live, and the three composers drift apart.

**Independent Test**: `postio-body` round trips:
- `to_document(from_document(d)) == d` for every construct;
- `parse(editor_html(d)) == d`;
- the draft's stored HTML read back equals the document.

The terminal composer's format commands are TUI unit cases. The Mac's
`ComposeCommands` claims every new mark (its existing Swift test).

**Acceptance Scenarios**:

1. **Given** Markdown with a table, a task list and a fenced block on the
   clipboard, **When** `mod+alt+v` is pressed, **Then** all three arrive as
   their constructs at the caret.
2. **Given** the same clipboard and `mod+v`, **Then** the text arrives
   literally.
3. **Given** a draft with every construct saved in GTK, **When** it is
   opened in the terminal composer, **Then** the Markdown shown reads back
   to the same document, with the code language and task states kept.
4. **Given** the terminal composer, **When** `alt+t` is pressed on a line,
   **Then** the line gains `- [ ] `.

---

### User Story 7 - Write any construct by switching to Markdown (Priority: P1)

**Now** (D8): the interim editing surface.

A person writing a reply in GTK wants a table and a task list.
1. They press `mod+alt+s`. The draft becomes Markdown text, with the
   caret on the same word, and the quote is a chip at the end.
2. They type the table's pipes and `- [ ] call the vendor`, and press
   `mod+alt+s` again.
3. The rich view shows the table and the task. The caret is in the task's
   text. The quote is untouched.

One undo returns the draft to before the switch.

**Why this priority**: It is how GTK and Mac users get every construct
before the native composer exists. It is the maintainer's Q1 answer, and
it reuses the round trip the terminal needs anyway.

**Independent Test**:
- **Model, at `--lib`:**
  - a property case: `to_document(from_document(d)) == d` over generated
    normal-form documents (FR-024);
  - the source maps on every construct, for the caret.
- **The toggle itself:** a `widgets_suite` case
  (`composer_markdown_source.rs`) with real keys, plus
  `storyboards/compose/markdown-source.toml`, red on `main`.

**Acceptance Scenarios**:

1. **Given** a draft holding every construct in scope, **When** it is
   switched to Markdown and back with no edit, **Then** the document is
   identical.
2. **Given** the caret after "Wed" in "Wednesday" inside a struck run,
   **When** the view switches to Markdown, **Then** the caret is after
   "Wed" in `~~Wednesday~~`. Switching back puts it after "Wed" again.
3. **Given** the caret between the two asterisks of `**`, **When** the
   view switches to rich, **Then** the caret is at the start of the bold
   text.
4. **Given** a reply, **When** it is switched to Markdown, **Then** the
   quote is one chip line that cannot be edited. Deleting it and undoing
   brings it back, and switching back keeps it where it was.
5. **Given** a table row with more cells than its header, typed in the
   view, **When** the view returns to rich, **Then** the extra cells are
   dropped, a notice line says so, and Undo restores the text.
6. **Given** the view open, **When** `mod+b` is pressed over a selection,
   **Then** the selection is wrapped in `**`.
7. **Given** a draft saved while in Markdown, **When** it is reopened in
   GTK, on the Mac or in the terminal, **Then** it opens as the same text,
   in the Markdown view.
8. **Given** the rich view, **When** `mod+shift+s` (Strikethrough) is
   pressed, **Then** the composer says "Strikethrough is in the Markdown
   view for now · ctrl+alt+s", and nothing else changes.

---

### User Story 8 - Attach a remote image on purpose, and only then (Priority: P2)

**Now** (D8), in its row form.

A person writing in Markdown types `![Q3 chart](https://example.com/q3.png)`.
1. Back in rich text, nothing has been fetched. The attachment list shows
   "Not attached yet · Q3 chart · from example.com", with "Download and
   attach ctrl+alt+i".
2. They press it. The image downloads once, and it appears in the body and
   the attachment list as an inline image.
3. Had they not pressed it, the message would have gone with a link.

**Why this priority**: It is the maintainer's Q3 answer. It is P2 because
Insert image, paste and drop already cover attaching an image.

**Independent Test**:
- **The fetch, against a fake fetcher.** `postio-session` cases count the
  calls:
  - zero across typing, the source view, Paste as Markdown, draft
    open, autosave, draft sync and send;
  - exactly one per press.

  The failure states are covered the same way. No test reaches the
  network.
- **The row:** a `widgets_suite` case.

**Acceptance Scenarios**:

1. **Given** `![x](https://example.com/x.png)` in the draft, **When** the
   draft is saved, reopened, synced to the server's Drafts and sent
   without a press, **Then** no request is made, and the sent HTML has
   `<a href="https://example.com/x.png">x</a>`.
2. **Given** the press, **When** the server returns a 40 KB PNG, **Then**
   the draft carries it as a `cid:` inline image, and exactly one request
   was made.
3. **Given** the press, **When** the server returns `text/html`, an SVG,
   or 11 MB, **Then** nothing is attached and the row names the reason.
4. **Given** the press while offline, **Then** the row says so. Going
   online later makes no request until the next press.
5. **Given** the row's remove button, **Then** the chip becomes a plain
   link in the body and its row goes.

---

### Edge Cases

- **A sequence inside inline code or a code block** never fires. Markdown
  does not format inside code either.
- **A sequence inside a carried reply quote** (`Block::Quoted`) never
  fires, and the commands are unavailable there with the reason "The
  quoted message is kept as it was sent". ADR 0033's fidelity is the point.
- **Intraword underscores** never open or close emphasis; intraword
  asterisks do, as CommonMark says.
- **`~` alone** (`~5 minutes`) is never strikethrough while typing; only
  `~~` is. Pasted and terminal Markdown follow GFM, which accepts both.
- **An ordered list starting at 0** keeps 0. A start of more than nine
  digits is not a list, as CommonMark says.
- **A heading in a list item or a quote** is allowed, as CommonMark allows
  it. A table inside a table is not.
- **A ragged table** (rows with more or fewer cells than the header) is
  padded with empty cells or truncated, as GFM says. Nothing is lost
  silently: truncated cells are reported by `narrow`'s `Lost`.
- **A table cell holds inlines only.** Enter in a cell moves to the cell
  below (or adds a row from the last row), and Shift+Enter is a line break
  inside the cell.
- **A code block's language** that is not a valid word (spaces, quotes) is
  kept as the fence's text in the terminal and dropped from the label
  elsewhere. It is never put into HTML except as the escaped `data-`
  value.
- **Pasting rich HTML** keeps what the widened subset can hold: `<del>`,
  `<s>` and `<strike>`; `<h4>`–`<h6>`; simple tables; `<ol start>`; and a
  GitHub-style checkbox list (`<li><input type=checkbox checked>`) as
  tasks. Everything else narrows as today.
- **A remote image typed as Markdown** becomes a chip that fetches only
  when pressed, and is sent as a link otherwise (D7).
- **The source view and a carried quote**: only `Block::Quoted` has no
  Markdown, and it is an atomic chip line in the view.
- **Undo** reverses a conversion in one step and a command in one step
  (spec 002 FR-070, ADR 0004 Q5's `EditHistory`).
- **Reduced motion and speed**: nothing here animates. A sequence converts
  within the 16 ms interaction budget, measured as work done per keystroke
  (no store reads at all).

## Requirements *(mandatory)*

### Functional Requirements

**The document**

- **FR-001**: The composer's document MUST hold every construct in "Every
  construct": headings of six levels, strikethrough, ordered lists with a
  start number, task items with a done state, code blocks with an optional
  language, and tables with a header row, body rows and a per-column
  alignment (none, left, centre, right). It MUST stay a closed set with no
  styling the user controls (ADR 0004, D5).
- **FR-002**: `parse` MUST stay total. It MUST read the HTML this spec emits
  back into the same document, and narrow anything else as today. In
  particular, a table with merged cells or block content in a cell MUST
  narrow rather than be forced into a grid.
- **FR-003**: Opening a saved draft MUST give back the document that was
  saved, every construct included, with no storage migration (drafts store
  the generated HTML).

**Commands and keys**

- **FR-004**: Every construct MUST be reachable from a registry command
  with the default key and terminal alternate given in "Every construct",
  in the `Composer` context. The table picker's commands MUST be in a
  `Table picker` context. Every one MUST be rebindable from `[keys]`, and
  listed in the command bar, the key map and `docs/keybindings.md`
  (Constitution II).
- **FR-005**: No new key MAY collide with a key already in force in the
  `Composer` context or `Everywhere`. The registry's collision test MUST
  prove it.
- **FR-006**: Every formatting command MUST be in the Mac's Format menu,
  and MUST be claimed by the Mac's `ComposeCommands`.
- **FR-006a**: A command MUST be registered only once something answers
  it. The registry rows follow `data-model.md`'s Phase column (D8).

**Typed sequences** (the table and its contract now; the rich surface's implementation held for ADR 0039)

- **FR-007**: Every typed sequence in "Every construct" MUST be a row of
  `postio_ui::editor::markdown::SEQUENCES`, reaching only a formatting
  command (spec 002 FR-068, unchanged). The GTK and Mac surfaces MUST
  implement exactly that table.
- **FR-008**: The table MUST grow a third trigger for sequences that
  complete on Enter: setext underlines, code fences, dividers and table
  delimiter rows. It MUST grow a fourth for sequences that complete on the
  next space or punctuation: bare autolinks.
- **FR-009**: A conversion MUST be undone by one undo, restoring the literal
  characters, unconverted (spec 002 FR-070).
- **FR-010**: No sequence MAY fire inside inline code, a code block or a
  carried reply quote. An escaped marker MUST NOT pair with any later one.
- **FR-011**: Emphasis with `_` MUST NOT open or close inside a word.

**The formatting picker and toolbar** (held for ADR 0039)

- **FR-012**: The `formatting` command and the toolbar's last button MUST
  open the formatting picker described in "The formatting picker", built on
  spec 007's one picker pattern (FR-092). It MUST name what it acts on,
  filter as the person types, show each row's key and typed sequence, mark
  what is in force at the caret, and close on `Esc` with nothing changed.
- **FR-013**: The toolbar MUST gain Strikethrough, Task list, Inline code
  and Formatting, each in the shared icon button with its key in its
  tooltip. At any width the Formatting button MUST stay visible.
- **FR-014**: The surface MUST report which of the new commands are in force
  at the caret, through the same reflection channel bold and italic use, so
  the toolbar and the picker show it.

**Sending**

- **FR-015**: The HTML sent for each construct MUST be the one in "Every
  construct". The `harden` backstop MUST admit exactly those elements and
  attributes, and MUST change nothing over any document the serialiser can
  produce (its existing property).
- **FR-016**: A task list MUST NOT be sent as `<input>`, and its state MUST
  be legible with all styles stripped.
- **FR-017**: The plain-text alternative MUST follow "Every construct"
  (D6). It MUST stay `format=flowed`. Code blocks and table rows MUST be
  fixed lines, never reflowed.
- **FR-018**: A message whose document is plain text MUST still be sent as
  `text/plain, format=flowed` alone.

**Paste and the other composers**

- **FR-019** *(held for ADR 0039)*: `paste_markdown` ("Paste as Markdown",
  `mod+alt+v`, `alt+V`) MUST convert the clipboard's text through
  `markdown::to_document` and insert the result at the caret. Ordinary
  paste MUST keep today's behaviour. Until it is built, pasting into the
  Markdown source view is the way to paste Markdown.
- **FR-020**: The terminal composer MUST read and write every construct as
  Markdown (`to_document` and `from_document`, with GFM's tables,
  strikethrough and task lists enabled). Its format commands MUST insert
  the markers for the new commands it offers.
- **FR-021**: A remote image MUST NOT be fetched except by FR-031's press
  (D7).

**The Markdown source view** (now)

- **FR-023**: `toggle_markdown_source` ("Edit as Markdown or rich text",
  `mod+alt+s`, `GRAPHICAL_MAIL`) MUST switch the GTK and Mac composers
  between the rich view and a plain-text view of the same draft as
  `from_document`'s Markdown. The Markdown view MUST be unmistakable: a
  strip naming it and its key, and the toggle shown pressed.
- **FR-024**: For every document in normal form,
  `to_document(from_document(d)) == d` MUST hold, for every construct in
  scope. A property test over generated documents is the proof. Shapes
  CommonMark cannot spell MUST be written with the spellings in "The
  Markdown source view", and MUST be read back.
- **FR-025**: A `Block::Quoted` MUST appear in the view as one atomic,
  read-only line. It MAY be deleted whole, MUST NOT be editable or
  forgeable, and MUST be restored at its line on return.
- **FR-026**: On return, anything the typed Markdown narrowed MUST be
  reported in a notice line with Undo. Nothing may be dropped silently.
- **FR-027**: The caret MUST land on the same character of text on each
  switch, by the rules in "The Markdown source view". A selection MUST
  keep both ends where both map.
- **FR-028**: A session in the view MUST be one step in the draft's undo
  history. A draft saved from the view MUST keep the typed text in
  `body_markdown`, and MUST reopen in the view in every composer.
- **FR-029**: In the view, every formatting command offered now MUST edit
  the Markdown through `postio_ui::editor::markdown_edit`, the functions
  the terminal composer uses. In the rich view, a formatting command not
  yet built there MUST say so, with the view's key, and MUST change
  nothing.

**Remote images** (now)

- **FR-030**: Markdown naming an `http` or `https` image MUST become a
  remote image (a chip). HTML pasted into the rich view MUST keep
  narrowing a remote `<img>` away.
- **FR-031**: The image MUST be fetched only in response to
  `attach_remote_image` (`mod+alt+i`, `alt+I`), or the chip's button,
  pressed for that chip. The press MUST make exactly one GET, under the
  limits in "Remote images". It MUST NOT block the UI.
- **FR-032**: Nothing else MAY cause a request for a remote image: not
  typing, not opening or switching views, not autosave, not draft sync,
  not send, not a retry. There MUST be no prefetch, DNS lookup or HEAD
  request.
- **FR-033**: A fetched image MUST be attached through the existing inline
  image path, as a `cid:` part, only if its bytes are PNG, JPEG, GIF or
  WebP and it is at most 10 MB.
- **FR-034**: Each failure (unreachable, too large, not an image, offline)
  MUST leave the chip, name the reason, and offer the key again. Nothing
  MAY be retried or queued without a press.
- **FR-035**: A chip that is not attached when the message is sent MUST be
  sent as a link (HTML `<a href>alt</a>`, plain text `alt <url>`). The
  composer's warning line MUST say so before Send. Logs MUST carry the
  draft id, the outcome and the byte count, never the address.

**Product truth**

- **FR-022**: `docs/PRODUCT.md` §10 MUST describe this formatting scope,
  the formatting picker and the plain-text rule, and MUST cite this spec.

### Key Entities

- **Document**: the draft's body. It gains the constructs in FR-001. It
  stays closed, has no styling, and is the record (ADR 0004 Q3).
- **Sequence**: one row of the typed-sequence table: a marker or pattern,
  its trigger, and the command it reaches.
- **Formatting command**: a registry entry in the `Composer` context. It has
  a title, a default key, a terminal alternate, an icon, and its typed
  sequence shown in the picker.
- **Remote image**: an image named by an `http(s)` address, not yet
  attached. It is shown as a chip, sent as a link, and becomes an `Image`
  when its one fetch succeeds.
- **Source map**: the correspondence between character offsets in
  `from_document`'s Markdown and positions in the document, both ways.
  It is used only to place the caret.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: When the now phase lands, every CommonMark and GFM construct
  in scope can be authored in every composer through Markdown: the
  terminal's text, or GTK's and the Mac's source view. When the held phase
  lands, each can also be typed live and reached by a key and the picker.
- **SC-002**: A document holding every construct survives each of these
  round trips with nothing lost:
  - sent HTML read back;
  - the stored draft reopened;
  - Markdown in the terminal and back.
- **SC-003**: A message holding every construct is readable with styles
  stripped and in its plain-text part. A reviewer can read the done state of
  every task, the struck words, the code and every table cell.
- **SC-004**: A typed conversion costs the same order of work as today's
  `**bold**`. It reads nothing from the store, and is under the 16 ms
  interaction budget on the reference machine.
- **SC-005**: A first-time user finds any construct through the picker in
  one search, by typing part of its name.
- **SC-006**: Exactly one composer gesture causes a network request: the
  press of a remote image's chip, one request per press.

## Assumptions

- #1485 lands a heading command before or during this branch. D3's reshape
  is this spec's first task either way.
- The rich editing work waits for ADR 0039's native composer (Q4 = C).
  Until then, the WebKit composer shows what the source view writes
  through `editor_html`, drawn by WebKit's defaults, and edits nothing
  new.
- The fetch uses Pimalaya's `io-http`, already in the tree for account
  autoconfiguration (`postio-account`).
- `pulldown-cmark` 0.13, already a dependency of `postio-body`, parses GFM
  tables, strikethrough and task lists behind options. The Pimalaya family
  has no Markdown crate; the survey is recorded in `research.md`.
- Recipient-client behaviour (Gmail, Apple Mail, Outlook, Thunderbird, mutt)
  is from the clients' documented HTML support. The quickstart's manual
  send to the maintainer's own test accounts confirms it once. No automated
  test reaches the network.
