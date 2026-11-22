# Feature Specification: Compose formatting — everything Markdown can say

**Feature Branch**: `feature/compose-formatting`

**Created**: 2026-11-22

**Status**: Draft — for the maintainer's review before any task starts

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

**D1. Markdown stays the way in, not the format.** A draft is a `Document`.
The maintainer's 2026-08-24 rejection of a Markdown-authored composer
(ADR 0003, restated by ADR 0039) is not reopened by this spec; Q1 asks
whether they now want a Markdown source view alongside it.

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
command bar and not a panel. See "The formatting picker" and Q2.

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

**D7. Typed remote images are never fetched.** This is unchanged from
today. Q3 asks whether the maintainer wants an explicit way to attach one.

**D8. The model, the input, the send path and the terminal come first, the
surface last.** The document, `parse`, `to_html`, the text alternatives,
`markdown::to_document` and `from_document` are surface-independent. So is
the sequence table in `postio_ui::editor::markdown`. All of it survives
ADR 0039. The WebKit surface gets the input and the rendering on top. Q4
asks whether the surface should wait for ADR 0039.

**D9. Vocabulary.** The UI says "Divider" (not "horizontal rule" or
"thematic break"), "Task list" and "Done" (not "checklist" or "checked"),
"Code block" and "Inline code", and "Strikethrough". These are new words
for the app, recorded here as the vocabulary decision the ux-architect
skill asks to be named.

## Clarifications

Three questions only the maintainer can answer. Each one names the option
the spec assumes, so planning is not blocked; an answer that differs
changes the tasks it names.

### Q1 — Does "everything I can do on a Markdown file" include editing the Markdown itself?

[NEEDS CLARIFICATION: is a Markdown source view wanted in the rich composers?]

| Option | Answer | Implications |
|---|---|---|
| A *(assumed)* | No. Every construct is reachable by typing, the picker and keys; the draft is rich text | This spec as written. No mode |
| B | Add `toggle_markdown_source`: the same draft as Markdown text, editable, read back on toggle (the terminal's `from_document`/`to_document` round trip) | One new command and a mode to make unmistakable; lossless except a carried reply quote, which stays a block. About one user story more (US8) |
| C | Markdown becomes the draft's stored form | Reverses ADR 0003 and the 2026-08-24 decision; reopens ADR 0004's model. Out of this spec |

### Q2 — What should the "formatting window" be?

[NEEDS CLARIFICATION: confirm the formatting picker over the alternatives]

| Option | Answer | Implications |
|---|---|---|
| A *(assumed)* | A picker anchored at the caret (`mod+alt+f`, and the toolbar's last button), listing every construct with its key and Markdown, filtering as you type | Spec 007's one picker pattern; no new kind of surface |
| B | The command bar (`mod+k`) opened on its Format group | No new surface at all, but the bar sits in the window's header, far from the caret, and cannot show what is active at the caret |
| C | A second toolbar row, shown or hidden with a key | Everything visible at once; it costs a row of the composer on every message, and it is a mode |

### Q3 — A typed `![alt](https://…)`: what happens?

[NEEDS CLARIFICATION: should a typed remote image be fetchable on request?]

| Option | Answer | Implications |
|---|---|---|
| A *(assumed)* | It stays the literal text, never fetched; images come from Insert image, paste and drop, as `cid:` parts | Today's rule (ADR 0003); nothing new leaves the machine |
| B | It becomes a chip offering "Download and attach" (one fetch, on that press, then a `cid:` part) | The user asks for exactly one request; it needs a network path from compose that does not exist today |
| C | It becomes a link to the address | No fetch; the recipient sees a link where the writer typed an image |

### Plan-level question

**Q4** (sequencing against ADR 0039) is in `plan.md`. It changes the order
of the work, not the behaviour.

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
| Image | `insert_image` · `mod+shift+g`, `alt+g` *(exists)* | `![alt](cid:…)` in the terminal; a remote address stays literal (D7, Q3) | Inline, at its size, capped to the column | `<img src="cid:…" alt>` in `multipart/related` | `[image: alt]` |
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

The surface the maintainer called the formatting window (D4, Q2).

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

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Type the inline constructs and get them (Priority: P1)

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
- **A remote image typed as Markdown** stays its literal text (D7, Q3).
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

**Typed sequences**

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

**The formatting picker and toolbar**

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

- **FR-019**: `paste_markdown` ("Paste as Markdown", `mod+alt+v`, `alt+V`)
  MUST convert the clipboard's text through `markdown::to_document` and
  insert the result at the caret. Ordinary paste MUST keep today's
  behaviour.
- **FR-020**: The terminal composer MUST read and write every construct as
  Markdown (`to_document` and `from_document`, with GFM's tables,
  strikethrough and task lists enabled). Its format commands MUST insert
  the markers for the new commands it offers.
- **FR-021**: A typed or pasted remote image MUST NOT be fetched (D7).

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

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every CommonMark and GFM construct in scope can be authored
  in each composer in at least two ways: by typing its Markdown and by a
  key. All but the escape can also be authored from the picker.
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
- **SC-006**: No composer gesture causes a network request.

## Assumptions

- #1485 lands a heading command before or during this branch. D3's reshape
  is this spec's first task either way.
- The WebKit composer surface is still the shipped one when the surface
  tasks run (Q4). Its `execCommand` vocabulary has no table or task
  operations, so those edits are done in Rust on the `Document` and
  re-rendered, which is ADR 0039's step 2 for these commands.
- `pulldown-cmark` 0.13, already a dependency of `postio-body`, parses GFM
  tables, strikethrough and task lists behind options. The Pimalaya family
  has no Markdown crate; the survey is recorded in `research.md`.
- Recipient-client behaviour (Gmail, Apple Mail, Outlook, Thunderbird, mutt)
  is from the clients' documented HTML support. The quickstart's manual
  send to the maintainer's own test accounts confirms it once. No automated
  test reaches the network.
