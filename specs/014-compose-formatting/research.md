# Research: Compose formatting

Each entry is a decision, why it was taken, and what else was weighed.
"Today" means `main` at 0f160a97.

## R1 — Parse Markdown with `pulldown-cmark`'s GFM options

**Decision.** `markdown::to_document` turns on three of `pulldown-cmark`
0.13's options:

- `ENABLE_TABLES`;
- `ENABLE_STRIKETHROUGH`;
- `ENABLE_TASKLISTS`.

It maps the new events (`Tag::Table`, `TableHead`, `TableRow`,
`TableCell`, `Tag::Strikethrough`, `Event::TaskListMarker`, a
`CodeBlockKind::Fenced` info string, and `Tag::List(Some(start))`) to the
new document shapes (`data-model.md`). GFM's extended autolinks are not in
`pulldown-cmark`. They come from R2's function, run over the text the
parser hands back.

**Why.** The crate is already a `postio-body` dependency
(`default-features = false`), and it parses CommonMark exactly. The options
are its GFM layer, so they add no new dependency.

**Alternatives.**
- `comrak`: full GFM, autolinks included. Rejected: it is a second Markdown
  parser in the graph, with its own AST to keep in step.
- `markdown-rs`: rejected for the same reason.

The issue-1722 worktree bumps `markdown.rs`'s neighbours for a reader-stack
upgrade. It does not touch `pulldown-cmark`; this branch rebases over it
when it lands.

## R2 — GFM autolinks are one pure function in `postio-body`

**Decision.** `postio_body::autolink` finds GFM's extended autolinks
(spec 0.29, §6.9) in a run of text:

- `www.` and `http(s)://` with a valid domain;
- email addresses, which become `mailto:`.

It applies GFM's trailing-punctuation and parenthesis-balance rules, and
every result goes through `Href::parse`. Two callers use it:
- the typed-sequence engine, on the space or punctuation that ends a word
  (R5);
- `to_document`, over every text event outside code and links.

**Why.** It is the one GFM construct the parser does not cover. It is
small, rule-shaped and red in a second as a unit case.

**Alternatives.** The `linkify` crate. Rejected: it is a new dependency,
and it does not follow GFM's punctuation rules, so pasted and typed
autolinks would disagree with GitHub's rendering of the same text.

## R3 — What each construct is on the wire

The recipient's client is not a browser. The constraints we design for:

| Client | Constraint that matters here |
|---|---|
| Gmail (web and apps) | Removes `<input>`; keeps `<style>` in the head only for some accounts. Inline `style` is kept; `class` is useless without a stylesheet. `<del>`, `<s>`, `<pre>`, `<code>` and `<table>` attributes are kept |
| Outlook for Windows (Word engine) | Honours `border`, `cellpadding`, `cellspacing` and `align` on table cells. Ignores `list-style-type` on some versions; ignores `max-width` |
| Apple Mail, Thunderbird | Render HTML faithfully; `<input>` is drawn but inert |
| mutt, aerc, a screen reader, a list digest | Read `text/plain`, so the plain part is the message |
| Postio's own reader | Its sanitiser refuses `<input>` (privacy), admits table layout attributes (`TABLE_LAYOUT`), `<del>`, `<s>` and `h1`–`h6` |

**Decisions.**

- **Strikethrough → `<del>`.** GFM's own rendering. `parse` also reads `<s>`
  and `<strike>`.
- **Task list → `<ul style="list-style-type:none">`**, each item starting
  with `☐ ` (U+2610) or `☑ ` (U+2611). The list and each done item carry
  `data-postio-task` (`open` or `done`) for the round trip.
  - Rejected: `<input type=checkbox disabled>` (GitHub's), because Gmail
    and Outlook drop it and Postio's own reader refuses it.
  - Rejected: an image per box, because it adds an attachment per item.
  - Where the style is stripped, a bullet shows before the box, which is
    still readable.
- **Table → `<table border="1" cellpadding="6" cellspacing="0">`**, with a
  `<thead>` row of `<th>`, a `<tbody>`, and `align="left|center|right"` on
  each cell of an aligned column.
  - Attributes rather than inline CSS because the Word engine honours
    them, and Postio's sanitiser and `harden` already admit them
    (`TABLE_LAYOUT`).
  - Rejected: no attributes at all, because tables then arrive without
    borders in Gmail and Outlook and read as loose words.
- **Code block → `<pre data-postio-lang="rust">`**, its text escaped.
  - Rejected: `<pre><code class="language-x">`, because ADR 0004 forbids
    `class` and no client highlights it anyway.
  - The language travels for the draft's round trip, and in the plain
    part's fence where a person can read it.
  - Long lines are not wrapped in the HTML: `<pre>` scrolls in every client
    that matters, and code must not wrap silently.
- **Headings → `<h1>`–`<h6>`.** Every client draws them.
- **Ordered start → `<ol start="N">`.** Every client honours it.
- **Image → `<img src="cid:…" alt>`** in `multipart/related`. This is
  today's path (#341, #1571), unchanged.

`harden`'s `EMITTED_TAGS` gains `h4`, `h5`, `h6` and `del`. Its attribute
list gains `start` on `ol`, and `data-postio-lang` and `data-postio-task`
on `pre`, `ul` and `li`. `style` is already admitted generically.
`tests/body_suite/outgoing.rs`' "the backstop changes nothing" property
covers the new shapes by construction, once the generator in its property
test is widened.

## R4 — The plain-text alternative (D6)

**Decision.** `to_text` and `to_flowed_text` render:

| Construct | Plain text |
|---|---|
| Bold, italic | the words (unchanged; spec 002 FR-072) |
| Strikethrough | `~~words~~` |
| Inline code | the words |
| Heading 1 | the text, then `=` repeated to its width |
| Heading 2 | the text, then `-` repeated to its width |
| Heading 3–6 | `### text` … `###### text` |
| Task item | `- [ ] text` / `- [x] text` |
| Ordered list | numbered from its start |
| Code block | ```` ```lang ````, the lines verbatim, ```` ``` ```` |
| Table | a pipe table, cells padded to each column's widest, with an alignment row (`:--`, `:-:`, `--:`, `---`) |
| Divider | `----` (unchanged) |

**Why.** A plain-text reader is often a Markdown reader by habit. Dropping
`~~` inverts meaning, and dropping `[x]` loses the state. Setext
underlines read as headings to anyone and are valid Markdown, which is why
H1 and H2 use them rather than `#`.

**Flowed.** Every line of a code block and every table row is a *fixed*
line, never reflowed. It is space-stuffed where it starts with a space,
`>` or `From `, as `Block::Pre` already is. A table row ends in `|`, so it
can never be read as a soft break. Rows wider than 78 columns stay whole:
RFC 3676 allows 998.

`to_search_text` follows `to_text` but drops every marker. An index wants
the words, as `Links::LabelOnly` already argues.

**Alternatives.**
- The plain part as `from_document`'s Markdown, verbatim. Rejected:
  `**bold**` in plain text is the doubled marker spec 002 FR-072 forbids.
- Today's rule (all markers dropped). Rejected for the meaning-changing
  cases above.

## R5 — The typed-sequence engine

`postio_ui::editor::markdown::SEQUENCES` stays the one table. Both WebKit
surfaces run it (generated into `editor.js` by `markdown_table_js`), and
it remains the contract the ADR 0039 surface will implement.

**Decision.** It grows from two triggers to four:

| Trigger | Fires on | Rows |
|---|---|---|
| `LinePrefix` (exists) | the space after a marker at a block's start | `#`…`######`, `-`, `*`, `+`, an ordered pattern, `>`, `[ ]`, `[x]`, `- [ ]`, `- [x]` |
| `Wrapping` (exists) | the closing marker | `**`, `__`, `*`, `_`, `~~`, a backtick run, and the link's `)` and `>` |
| `LineComplete` (new) | Enter, when the block reads exactly the marker | `===` (setext H1), `---` (setext H2 when on the line under one paragraph line, else a divider), `***`, `___`, ```` ``` ````/`~~~` plus an optional language, a table delimiter row |
| `WordComplete` (new) | the space or punctuation that ends a word | a bare autolink (R2) |

- A marker becomes a pattern where it has to: the ordered marker is
  `digits{1,9}` then `.` or `)`; the fence carries a language; the
  delimiter row carries cells. Patterns are a small enum, not a regex
  engine, so the table stays enumerable and testable.
- **Intraword `_`.** A `_` run is an opener only when the character before
  it is not alphanumeric, and a closer only when the character after it is
  not. CommonMark's left- and right-flanking rules are reduced to this,
  which is all a live typist meets.
- **Escapes.** A marker character typed right after `\` replaces the pair
  with the character and records its offset as escaped in the block. The
  opening-run search skips escaped offsets. The bridge keeps the record
  per block and shifts it on edits within the block. Under ADR 0039 the
  escape becomes a property of the edit intent instead, and the
  per-keystroke cost is unchanged.
- **Where nothing fires.** The recogniser returns early when the caret is
  inside `<code>`, `<pre>` or a `data-postio-quoted` element.
- **Longest first**, now across patterns too. The existing ordering test
  generalises: `***` before `**` before `*`, and `- [ ]` before `-`.

The test `no_heading_sequence_until_there_is_a_heading_command` is retired
by #1485. The test `every_sequence_reaches_a_formatting_command_and_nothing_else`
grows its list of formatting commands, which is the FR-068 bound, still
enforced.

## R6 — Edits `execCommand` cannot make

WebKit's `execCommand` has bold, italic, strikethrough, lists, `formatBlock`
(`h1`–`h6`, `pre`, `blockquote`), `insertHorizontalRule` and `indent`/
`outdent`. It has nothing for tables, tasks, list starts, code languages,
or inline code (`<code>` is not a `formatBlock` target).

**Decision.** For those, the command runs in Rust on the `Document`, as a
pure edit in `postio_body::edit_ops`:

- insert a table; add or delete a row or column; align a column;
- toggle a task, or toggle a task list;
- set a list's start;
- set a code block's language;
- toggle inline code over a selection.

The surface then reloads the block from `editor_html` and puts the caret
back by a block path (the block's index path and the offset in its text).
This is ADR 0039's step 2 ("route the existing composer's edits through
`apply`") for exactly these commands. It is the same shape as today's
`image_script`: Rust decides, the script performs.

The caret path is block-granular: a cell, an item, a code block. It does
not meet the IME risk ADR 0039 names, because no edit here runs during a
preedit. A command pressed mid-preedit commits the preedit first, which is
what WebKit already does for `execCommand`.

**Alternatives.**
- Hand-written DOM surgery in `editor.js` per operation. Rejected: it is a
  second implementation of each edit, in the language ADR 0039 is removing,
  and it is not unit-testable at the `--lib` tier.
- Waiting for ADR 0039. That is Q4.

## R7 — Keys

**Decision.** The keys in the spec's tables. How they were chosen:

- **Free in `Composer` and `Everywhere`** on today's registry (checked
  against `docs/keybindings.md`).
- **Conventions first.**
  - Google Docs: `mod+alt+0`–`6` for paragraph styles, `mod+backslash`
    for clear formatting, `mod+bracketleft`/`right` for indent.
  - GitHub: `mod+e` for inline code.
  - Postio's own shelf: `mod+shift+6` for task lists, beside `7`/`8`/`9`.
  - Docs' strikethrough is `alt+shift+5`, which collides with heading
    5's terminal alternate. Slack and Notion use `mod+shift+x`, which is
    Cancel send here. So strikethrough is `mod+shift+s`.
- **Clear of the desktop's own chords.** On GNOME that rules out:
  - `ctrl+alt+t` (terminal);
  - `ctrl+alt+Delete` (log out);
  - `ctrl+alt+arrows` (workspaces);
  - `ctrl+alt+l` (lock, on some distributions).

  On macOS it rules out `cmd+opt+esc` and `cmd+opt+d`. This is why the
  table operations live in the table picker, on digits, and not on
  `mod+alt+<arrow>`.
- **A terminal alternate** for every command the terminal offers, chosen
  so that `legacy_deliverable` accepts it. Commands the terminal cannot
  offer (`line_break`, `code_language`, the table picker's) take
  `GRAPHICAL_MAIL`.
- **`mod+alt+<letter>` on a Mac** is Command-Option, which types nothing,
  so it is safe in a text view. The `alt+<letter>` alternates have the
  existing composer alternates' Option-typing cost, no more.

The registry's collision test (`postio-core`) is the proof (FR-005). The
`docs/keybindings.md` generator test regenerates the reference.

## R8 — The heading command and #1485

#1485 is claimed and in progress, with no commits yet on 2026-11-22. It
proposes one `heading` command with a three-level cap. D3 needs every one
of six levels on one chord. To avoid racing it:

1. A comment on #1485 points at D3, so the implementer can land the
   seven-command shape directly.
2. If #1485 lands one `heading` command anyway, T004 reshapes it before
   any release ships the old id. The ids are a file format, and there are
   no installs to protect (no backwards compatibility).

## R9 — The formatting picker is the shared picker frame

**Decision.** `postio_widgets::widgets::pickers::Picker` (the frame
snooze, remind, label and move share) hosts the formatting, table and code
language pickers. It already reads its keys from `Context::Picker`.

The frame's `picker_choose_1`–`4` grow to `picker_choose_0`–`9`, so the
heading presets (`0`–`6`) and the table picker's six operations use the
same commands. The snooze, remind, label and move pickers have four
presets, so a fifth key chooses nothing there, as `5` does today.

The words (group titles, row titles, the "acts on" line, the empty and
unavailable reasons) live in `postio_ui::format_picker`, a pure module
held by unit tests. The GTK widget and the Mac's SwiftUI popover only
draw it.

**Alternatives.**
- A new popover type for formatting. Rejected: it is a second picker
  pattern, which spec 007 FR-092 forbids.
- The command bar's Format group. That is Q2 option B.

## R10 — The Mac

The Mac composes with a `WKWebView` running the same `editor.js` from
`postio_ui::compose::editor_script`. Its `ComposeCommands.marks` and
`handled` lists name every mark it claims, and `ComposeCommandsTests`
holds them against the registry. So:
- the Mac gets the sequences for free;
- it gets the Rust-side edits (R6) through new `postio-ffi` calls beside
  `mark_script`;
- its Format menu is built from `menu.rs`, which gains the new ids.

The picker is drawn natively in SwiftUI from `postio_ui::format_picker`,
as the Mac's other pickers are. The Mac cannot build GTK, and Linux cannot
build Swift, so each side's tasks run on its own machine. CI proves the
combination.

## R11 — Pimalaya survey (2026-11-22)

The Pimalaya family covers IMAP, SMTP, OAuth, JMAP, keyrings, process and
MML (`mml-lib`, Emacs's MIME Meta Language, which compiles to MIME). It
has no CommonMark or GFM parser and no HTML generator for mail bodies.
`mml-lib` writes MIME structure from a MML template; Postio already builds
MIME in `postio_model::outgoing`, and nothing here changes the MIME shape.
No Pimalaya crate applies.

## R12 — Constitution V: measuring a conversion

The budget is per keystroke, in the surface. A conversion reads nothing
from the store, so `counting` has no statement to assert.

The gate is structural:
- `SEQUENCES` is a fixed table;
- each recogniser looks only at the caret's text node, or for
  `LineComplete` at the caret's block and the one before it;
- a unit case pins that bound by counting the nodes visited on a fixture
  with a 10,000-word body.

The Rust-side edits (R6) are linear in the edited block, not the document,
and a unit case pins that too, on a document of 1,000 blocks.
