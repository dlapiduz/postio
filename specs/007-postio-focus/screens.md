# Screens: each build compared with its PNG

Spec FR-095 and SC-009. Each screen is rendered from the demo store
(`cargo run -p postio-gtk --example shot`, below) and read back beside its
reference in the maintainer's `Design/postio-focus-design/screens/` (and, for
the open message, `Design/focus-message-dialog/screens/`), in light and dark.
Every difference is written down here with its reason. The references are
never committed; they carry a real name until they are re-rendered.

Differences the spec decided are cited by their id in *Where the inputs
disagree* (C1–C27). Three apply to nearly every screen and are not repeated
row by row:

- **The demo store.** Names, dates, counts and folders are the demo's,
  anchored to the day the shot runs, not the reference's.
- **Key caps** are spelled by the shared hint code (C22): `ctrl+k`, `Escape`,
  `Return`, `J`, where the references draw "Ctrl K", "Esc", "↵" and "⇧J".
- **Hint lines** (the bulk bar's selection keys, a footer's keys) are one
  line of text from `postio_ui::hints::line`, not a row of caps.

## The comparison

| Screen | Differences | Reason |
|---|---|---|
| 01 Inbox, light | (a) the sync label's icon is a bare check, not a check in a circle; (b) a selected row shows the icon theme's check glyph, not a filled box; (c) the bulk bar's buttons are bordered, not flat labels with Archive raised; (d) the cursor is the accent outline alone, with no accent tint; (e) Decline and Snooze are bordered buttons like Accept, where the reference draws them flat; (f) an answered, cancelled or past invitation, which no reference draws, shows "Accepted"/"Declined", "Cancelled" or "Past" in dim ink where the buttons were; (g) a to-do offers Snooze `s`, and Task `t` beside it only with a vault; (h) the marker's chip, day and quote are in the system accent, not the reference's blue | (a) the icon theme's `emblem-ok-symbolic`; (b) the stock `checkbox-checked-symbolic`; (c) the shared `ActionBar`; (d) FR-091: the accent is the focus ring, a 2 px ring that tints nothing (contracts/focus-surface.md); (e) the row draws every answering action as one button kind; (f) FR-103, through `postio_ui::focus_row::marker_line`, and constitution II: every key has a visible button; (g) C9; (h) C26, FR-091 |
| 02 Inbox, dark | As 01. Libadwaita's own dark palette, with nothing of Focus's in between: the neutral selection and the hairlines lift rather than darken, and the markers, unread dots and cursor ring take the dark accent | As 01; FR-090 |
| 03 Has-action filter | As 01. The heading counts the rows it lists ("Has action · 8" over eight), where the reference heads eight rows "Has action · 7" | The heading and the toggle read one count (`FocusCounts.has_action`) |
| 04 Open message | Accepted (C25, C26): the chrome is Adwaita Sans and Mono, so the subject is Adwaita Sans 600 30/34 rather than Barlow Condensed, and the verbs are padded 6 px a side to fit 820; the card's fill, its tag and the body's links are the system accent. Otherwise: (a) in a 900 px window the dialog is 810 tall and 45 px down, not 820 and 40; (b) `O` and `E` rather than `⇧O` and `⇧E`; (c) the `k`/`j` steps are one linked pair with each cap inside its button; (d) dates read "Today, 15:22" for today's mail; (e) the bulk bar at the window's foot is not dimmed; (f) the render-mode line's mark is the U+25D1 glyph, not a drawn half-circle; (g) the fold line says how many lines it folds ("31 quoted lines"), not which message they quote; (h) Task and Note are not in the action row | (a) libadwaita's floating sheet keeps 5% of the window above and below (exact at 768: 688); (b) C22; (c) a key is taught inside the control it runs, and the pair stays compact; (d) the sender block's date rule; (e) it is outside the dialog host; (f) a glyph, not an image; (g) the fold knows its count, not its source; (h) C9: a to-do's card offers Task with a vault, and `t`/`n` open the capture sheet |
| 27 A newsletter on paper | As 04. Office mail on paper (shot 29) falls back from Calibri, which is not installed, to the renderer's sans. The shot files the newsletter in 04's row, so it keeps a thread marker, a label and a to-do card the reference's newsletter has none of | The fonts a sender names are not shipped; the demo store |
| 05 Compose | (a) suggestions open at four characters, "Grac", not three; (b) a suggestion row reads "Name <address>", with no "wrote 42 times", "list" or footer; (c) Cc and Bcc share one "+ Cc" control, not "Cc Ctrl ⇧C · Bcc Ctrl ⇧B"; (d) no Markdown toggle and no "Task after sending"; (e) the header is Detach, the title over "Plain text · N words · Draft saved locally …", and the X, with Send, Send later, Attach and Remind in an action row of their own and no footer; (f) From comes after To and Cc, and reads "Name <address>" with no ▾ for one identity; (g) no "Draft saved locally" until the first save lands; (h) a half-typed recipient earns "Grac in To does not look like an address" under the editor; (i) Send is a raised neutral button, not a dark fill; (j) the Labels row has no control of its own; (k) the list behind is not dimmed in the shot | (a) C23; (b) a `RecipientCandidate` carries only the address, not its sent count; (c) the composer's one `copy_fields` command; (d) C7, C9; (e) the composer is the message dialog with the composer's verbs ("The composer", below); (f) the shared composer's field order; (g) the subtitle says a save that happened; (h) the composer's recipient warning; (i) FR-091: no button wears the accent or a dark fill; (j) labels are added with the label picker; (k) libadwaita's dimming, under the shot's animations off |
| 06 Reply all | As 05 (c–g, i, k). (a) the quote is the composer's fold, "On 2026-09-28, … wrote:" over a closed "Quoted message", not "› On Sat 26 Sep, 15:22, … wrote · 18 quoted lines Ctrl ⇧Q show"; (b) the subtitle counts the quote among the words; (c) "1 To, 3 Cc" sits above the editor; (d) no reply is typed above the quote | (a) the shared editor's fold (`postio_ui::editor::document::fold_quotes`), opened by a click; the keymap has no command for Ctrl ⇧Q; (b) the quote is part of what is sent; (c) the composer's recipient count; (d) the shot does not type into a reply |
| 07 Search, plain English | Typed as "the invoice … sent last month" with the demo's correspondent. (a) "invoice" stays a free word, so its chip reads `invoice`, not `subject: invoice`; (b) chips are plain mono labels with no ×; (c) the saved searches carry no counts; (d) the "Search mail for …" row stays above the results, and the count sits in the heading ("Conversations · 3 matches") rather than at the right | (a) research R5: free text already searches subjects; (b) a chip is edited in the entry, after `Tab`; (c) a count per saved query is a query each, not read; (d) one results list for every kind of row |
| 08 Go to a folder | As 07 (b, c). `in:Rec` stays typed in the entry rather than becoming an `in: Receipts` chip. The rows have no address column | The bar completes `in:` to a folder and lists it, without rewriting what was typed |
| 09 Commands | (a) the command rows are the palette's titles, with no detail ("the focused message · Re: …") and no bold on the matched letters; (b) the blend also matches "arch" inside "Search" and "Saved search 1"; (c) `in:Archive` has no conversation count; (d) the search row sits under "Go to", with no "Search" heading and no ⇧↵ cap; (e) the mockup's "Archive everything read, older than a week" is not listed | (a)–(c) `postio_ui::finder::blend` and the palette as they are; (d) one list, with one heading per kind that has more than one row; (e) C11 |
| 10 Folders and labels | Labels have no counts. The Inbox mark is the icon theme's envelope (Adwaita has no tray), and Archive's a folder. No row is ringed on open; the first place is selected | A label count is a query per label, not read; the icon theme |
| 11 Snooze picker | (a) the title row names "3 conversations" where the reference names one conversation over the same three-row selection; (b) the number keys are framed caps, taller than the reference's bare digits, and the selected preset is tinted rather than ringed; (c) the times follow the demo's real clock, so on a Monday "Monday morning" and "Next week" name the same day | (a) US5: a picker acts on the selection when there is one, and says so; (b) the shared cap (C22) and the list's selection; (c) the demo is anchored to today. C14: "Later today", and "Tomorrow evening" after 6pm |
| 12 Remind picker | As 11. The placeholder reads "tue 9am" in both pickers, where 12 reads "thu 2pm"; the footnote's "No reply since" is dated today | One placeholder for the one date field |
| 13 Label picker | As 11 (a, b). With three conversations selected that share no label, no row says "✓ applied". Counts are conversations | "Applied" means on every conversation the picker acts on |
| 14 Move picker | As 11 (a, b). Recent has as many rows as the demo has destinations | The demo store |
| 15 Undo toast | The toast says "Archived 4 messages", not 3: the three selected conversations hold four messages. Its Undo button carries no keycap, and the toast has a close button | The engine counts the messages an action moved; AdwToast's button takes a label only, and its close button is libadwaita's |
| 16 Empty inbox | The tray is the icon theme's folder. "Inbox is empty" is the reading size, smaller than the reference's. The shortcuts are the shared ghost buttons, their labels bold where the reference's are regular | Adwaita has no inbox icon; the shared button kinds |
| 17 First sync | The progress bar runs the banner's full width under it, where the reference draws a short bar after the sentence | An AdwBanner holds a title and a button, nothing between; the bar is neutral, not the accent's fill (FR-091) |
| 18 Offline | "Retry now" sits at the banner's right edge, where the reference sets it beside the sentence | AdwBanner places its button at the end |
| 19 Sign-in error | As 18 | As 18 |
| 20 Key map | (a) one row per command, each with every binding, alternates included, where the reference merges pairs ("Top / bottom", "Extend selection") and shows one or two keys; (b) the groups hold every command Focus offers in the key map's contexts (zoom, find in message, the outbox's verbs, the picker's number keys among them), so the columns run past the dialog and scroll; (c) the footer names `[keys]` in `config.toml` | (a), (b) constitution II: the key map is generated from the registry and the groups table (`postio_ui::keymap_sheet`), and a command Focus offers is taught; merging pairs is a change to that table; (c) C3 |
| 21 Filtered | (a) the header's right holds "Sweep the inbox… F", where 21 has search and close; (b) the day heading counts the rows of that day read so far, which is the tab's count until more than a page is filtered in a day; (c) the focused row is the accent ring, as the list's cursor is; (d) the header says "Nothing here is deleted automatically · nothing here ever reached the inbox" | (a) FR-118: the sweep needs a control, and this is where filtering lives; (b) the view reads fifty rows at a time; (c) as 01 (d); (d) C4 |
| 22 Digest summary | (a) a statement is its own paragraph, with its reference as a plain trailing digit, where the reference draws one flowing paragraph per topic with boxed numbers inline; (b) the focused reference's message card sits once, after every statement, and reads only the subject, with no border, chip or date; (c) the `.focus-digest-summary-*` classes have no CSS, so nothing is bordered, coloured or ringed, as in the plain list; (d) the window has no footer of keys, as the plain list has none | (a), (b) a GTK label paragraph has no inline-widget text flow; (c), (d) as the plain list |
| 23 Email from a digest reference | (a) the header reads "Summary `Escape`" with the title and subtitle as drawn, but the toolbar (Reply, Forward, Archive, Note, Label, Unsubscribe, Stop digesting sender) is not drawn, only the banner and the body; (b) no "`j`/`k` next and previous source" hint, though both keys work; (c) the banner has no left border or tint | (a), (b) those verbs reach the message through the plain list, and US13's scenarios test the keys and the highlight; (c) as 22 (c) |
| 24 Digest this sender | (a) Create is the shared primary button, bordered, where the reference's is filled black; (b) the cadence and day are GTK drop-downs; (c) the list behind keeps its cursor on the row whose sender is digested, with no selection | (a) FR-091; (b) the toolkit's menus; (c) a selection would make one rule for every selected sender ("Digest these…") |
| 25 Obsidian capture | (a) Task and Note are a linked pair of toggle buttons, the chosen one grey, not a white segment on a grey track; (b) Add task is the shared primary button, raised and bold, not a dark fill; (c) the text is a plain entry, with no accent-ringed card; (d) "Due" has no "from “by Wednesday”"; (e) the quick picks are Today and the coming Monday, Wednesday and Friday in date order, then None, the chosen day ringed; (f) the suggestion reads "the subject names Harbor", not "… is linked from 23 notes in Harbor"; (g) the project line gives the note, with no "› ## Inbox" heading; (h) the Inbox row reads "Tasks.md (no project)", and "N open" counts only the tasks Postio captured; (i) the preview wraps at the dialog's width, and the footnote is below the fold; (j) the footnote says where the task goes, not what the row will show; (k) the preview puts the link before the date (C21) | (a), (b) the shared toggle and button kinds, and FR-091; (c) the entry is GTK's own; (d) a marker keeps the day it read, not the words (data-model.md, markers); (e) quick picks are relative to the day the sheet opens, and the mail's own day is always among them; (f) `postio-vault` suggests from the subject's words alone (`Reason::NamedInSubject`); (g) FR-180: a capture is appended to the end of the note; (h) `Vault::tasks` reads back only lines with a `postio://` link; (i) the dialog scrolls rather than grow past the window; (j) the row's "Task in … · due …" is not drawn yet (FR-181); (k) C21 |
| 40 Settings, Accounts | No reference: the handoff draws no Settings. Compared with the message dialog it borrows its frame from ("Settings", below). (a) the account's fields are the shared panel's, full-width entries and short port fields; (b) the find field takes the keyboard on open, so its focus ring is the one accent on screen; (c) the account's switch is the ink, not the accent | (a) the shared panel; (b) a person opening Settings is often looking for one thing; (c) FR-091 |
| 41 Settings, Sync & storage | One column, the rule running across; "Back up locally" lists each folder under Local store, below the fold at 1024x768 | The dialog is at most 820 wide ("Settings", below); ADR 0016's per-folder control |
| 42 Settings, Keyboard | Focus's commands only | Keyboard lists what Focus offers (`Section::shown_in`, `set_frontend`) |
| 43 Settings, Privacy | Every list says it is empty in its own words; the foot strip shows the shot's scratch path where a person's reads `~/.config/postio/config.toml` | The demo store; the strip names the file it writes |

## Interaction rules

**One close.** Every surface that closes (the window's top bar, the open
message, the composer, the digest, the raw source, the key map, Settings)
wears the one X icon button `postio_widgets::widgets::close_button()` builds,
at the right end of its header, after that header's verbs, with no keycap
(Escape closes, and the key map says so). A surface with a step back (the
digest's email page) has that at the left, worded "Summary". Pinned by
`close_buttons::every_closable_surface_has_the_same_x_at_the_right`.

**One icon button.** Every button that shows an icon and no words is the one
`postio_widgets::widgets::icon_button()` builds (`icon_menu_button()` for one
that opens a menu, `dress_icon()` for one whose icon is set elsewhere, such as
the composer's formatting toolbar): the ghost `.postio-icon-button`, 26 px,
its name as tooltip and accessible name, centred at its own size. The
constructor sets the centring, because a button left at GTK's default `Fill`
stretches into a tall pill in a 46 px bar. Pinned by
`icon_buttons::every_surfaces_icon_buttons_keep_their_own_shape`.

**Mouse and keyboard pairs.** Each row is an action a person expects from
either device.

| Surface | Keyboard | Mouse |
|---|---|---|
| List row | `j`/`k` move the cursor | a click moves the cursor |
| List row | `Return` opens | a double-click opens, through the same `OpenMessage` command |
| List row | `x` toggles selection | a press in the row's gutter toggles it |
| List row | `J`/`K` extend | Ctrl-click toggles and Shift-click ranges, through the commands the keys run |
| List row | the verbs' own keys | a right-click menu of the row's verbs ("The row menu") |
| List | `g g`, `G`, Page Up/Down | the scroll wheel and scrollbar |
| Open message | Escape closes | the X closes |
| Open message | `j`/`k` step | the step buttons step |
| Composer | Escape asks to close | the X closes |
| Key map, raw source, digest, Settings | Escape closes | the X closes |
| Pickers, command bar, row menu | Escape closes | a press outside closes |

## The open message

The message dialog's handoff (`Design/focus-message-dialog/SPEC.md`), with
the system font (C25) and the system accent (C26). The numbers live in
`postio_ui::focus_dialog`, pure functions a unit test runs.

**Size, from the window only.** `clamp(640, W − 2·max(96, 0.18·W), 820)`
wide (1024 → 655, 1280 → 819, 1440 and 1920 → 820) and the window's height
less 80, centred, radius 12, the list dimmed behind it (black at 20% light,
45% dark). It is refitted when the window resizes, never when `j`/`k` change
the message. Dimming and the corner are the `floating-sheet` rules every
Focus dialog shares (FR-092).

**Header, 52 px.** Left, the steps: one linked pair, each a quiet button with
its chevron and its key inside (`k`, `j`). Centre, the subject over "Message
5 of 60 · thread of 6" in mono. Right, the one X.

**Action row, 44 px, between hairlines.** Reply, Reply all, Forward,
Archive, Snooze, Remind, Label, Move and Delete, each with its cap, Delete's
reading `Del`. Below 760 px, or whenever the full row would not fit, Label,
Move and Delete fold into More (`.`, the registry's `more_actions`), its menu
in the row menu's dress. A draft being sent offers its own verbs instead
("Sending states").

**One centred column** for everything inside the message, sharing both
edges: `min(480, dialog − 96)` for a body in app colours, `min(640,
dialog − 48)` for one on paper, following the treatment the renderer decided
(and `O`). At 480, a line of prose is about seventy characters.

**Rhythm,** by explicit spacing (`focus_dialog::rhythm`): action row → thread
marker 28; marker → subject 12; subject → labels 10; labels → sender block
16; sender block padded 12 above and below, rows 22; sender → action card
12; card → notice 12; then 24 from the last of those blocks to the
render-mode line or the body; render-mode line → body 12; body →
attachments 24, a hairline, then 16; 32 at the foot. An absent block takes
its gap with it. Inside the body the rhythm has one home,
`postio-ui/data/treatment.css`: paragraphs 12 apart (a plain-text body's
blank lines part paragraphs rather than drawing an empty line), list items 4
apart under a 20 px indent, the attribution line 20 under the sign-off, the
28 px quote toggle 4 under it, and nothing under the last block.

**The notice** is the reader's one notice slot -- the list the message
came from with Unsubscribe `U` (T261), images held back with Show images
`i i`, reader view, a decode caveat; one at a time, the most important
first -- drawn as one more block about the message: 12 under the action
card, or under the sender block when there is no card, and the 24 to the
body (or its render-mode line) is then the notice's. It is unfilled like the
sender block, one line with a hairline under it, its icon and its button on
the column's edges. The unsubscribe notice shows only for a message that
really offers to leave a list: it has a `List-Id` or a `List-Unsubscribe`
(`postio_ui::unsubscribe::banner`). Personal mail has none and stays clean,
and outgoing mail never does. `U` is not tied to the notice: it leaves the
list on any message the shared offer rule allows (the `List-Id`, else the
sender's domain), band or not. With no notice the slot is hidden and takes
its 12 with it.

**Components.**

- **Subject**: Adwaita Sans 600, 30/34 (C25).
- **Labels**: 24 px pills with a hairline and an 8 px dot, then "+ Label".
- **Sender block**: hairlines above and below, no box; a 44 px column of
  muted 13 px field names; the name in bold with the address in mono; the
  date at the right in mono, "Today, 15:22" for today's mail.
- **Action card**: the only filled element. Radius 8, the accent at 8%
  (light) or 12% (dark), at least 48 px tall; an outlined tag, the date in
  mono, the sentence in italics, wrapping and never cut; Snooze `s` and
  Dismiss `-`, and Task `t` with a vault.
- **Keycaps**: mono 10.5, 16 px tall, a 1 px inset hairline, radius 4,
  muted, 6 px after the label.
- **Attachment chips**: 40 px, hairline, radius 8, the name over the size in
  mono. A chip opens the `o` chooser at its part.
- **Colours**: the handoff's surface, behind, ink, secondary and muted ink,
  hairline and strong hairline, light and dark, as `--postio-*` roles scoped
  to `.focus-open` (`focus-colours.css`), with libadwaita's accent (C26).

**Treatments.** Every HTML body is sanitised before it reaches the view
(scripts, forms, event handlers, remote content) and classified by
`postio_body::treatment::classify`:

- **Paper** when, after sanitising, it paints a page background (on body, a
  wrapper table, or most of the content; a near-white page is
  correspondence, not paper), has a fixed-width layout table of 480 px or
  more, or an image wider than 300 px. It is drawn exactly as sent on a
  white sheet with the light palette, radius 6 and a hairline edge, the
  sender's dark-scheme rules dropped. In dark mode the sheet is dimmed by an
  8% black veil. It is never inverted or recoloured. A layout wider than the
  column is zoomed to fit, down to 0.85 (`PAPER_FIT_FLOOR`), and scrolls
  sideways below that.
- **App colours** otherwise. `treatment::app_colours` strips `color`,
  `background`, `font-family`, `font-size`, `line-height` and `<font>`, and
  keeps bold, italic, underline, headings, lists, blockquotes, tables, links
  and inline images. `treatment.css` sets Barlow 15/24 in the column's ink,
  links in the accent, data tables in strong hairlines with 6/10 padding,
  images at most the column's width. A colour the sender kept on an inline
  element survives only at 4.5:1 against the surface (7 in high contrast),
  else it is the ink (`postio_render::theme::guard`). The body's ground and
  inks are read from the column's own roles through probes, so it is drawn
  on the dialog's surface in both schemes.

A quiet line above an HTML body (12.5 px, muted) names the treatment: "App
colours · sender colours and fonts removed · Show original O", or what the
paper trigger was. `O` switches the open message between the two, keeping
the column where it was; "Always for this sender" is kept with the per-sender
image allowances. A body that takes too long to lay out falls back to its
plain text, in the same column, face and rhythm, and the line says "Plain
text · this message took too long to lay out". The body is prepared off the
main thread.

**Find keeps the place.** `mod+f` opens find above the column, between the
action row and the message, so neither opening it nor its entry taking the
keyboard scrolls the message; the first match is the first from the top of
what is in view; `mod+g`/`mod+shift+g` step; Escape closes find before it
closes the message.

Pinned by `open_layout` (size, dimming, column edges, rhythm with and
without the notice, palette, More, the render-mode line, the card's
wrapping, `Del`, paper fit, the system faces) and `open_measure` (paragraph gap, measure, find, the column's
ground), with `postio-ui`'s `focus_dialog` cases and `postio-body`'s
`treatment` cases on the corpus.

## The composer

**The composer is the message dialog with the composer's verbs.** Pressing
Reply in the open message puts Send where Reply was.

- **Size.** The message dialog's rule: 820 × 820 at 1440 × 900, 655 × 688 at
  1024 × 768, following the window's resizes. In the pane, the pane's width.
- **Header bar, 52 px.** Left, Detach, an icon button that carries its key
  in its tooltip, not a cap, as the rarest verb here. Centre, the title
  ("New message", "Reply", "Reply to all", "Forward") over, in the mono
  subtitle, what will be sent and what has happened to it: "Plain text · 58
  words · Draft saved locally 16:12". Right, the one X. No verb reaches the
  title from either side, at any width.
- **Action row, 44 px between two hairlines.** Send `ctrl+↵`, Send later
  `ctrl+⇧+↵` with its ▾, Attach `ctrl+⇧+a`, and Remind `ctrl+h`, which reads
  "Remind · Tue 29 Sep" once a day is chosen ("if no reply" is its tooltip,
  accessible name and picker heading). Send is first, where Reply sits in the
  message dialog, and is the one primary button: plain raised with a bold
  label (FR-091). The others are the message dialog's quiet row verbs.
- **Keycaps: `hints::short`.** `Return` is `↵`, `shift` is `⇧` and `Delete`
  is `Del`; `ctrl` stays a word and the `+` stays. What is pressed and what
  a screen reader hears keep the binding's own names.
- **One column: the reader's.** The field rows, the formatting toolbar, the
  editor, the recipient warning and the attachments share one centred
  column, `min(480, dialog − 96)`, so a reply is written at the measure it
  will be read at.
- **Field rows.** To (with "+ Cc" at its right), Cc and Bcc when shown,
  From, Subject and Labels: 40 px rows, each with a hairline under it, the
  names in one label column as wide as the widest name, muted 13 px, the
  values in ink 12 px after it.
- **Formatting toolbar.** The composer's icon buttons, the shared 26 px
  ghost, in one row under the fields, its first button at the column's left
  edge. The paperclip and "Attach another" are hidden: Attach is a verb in
  the action row, and one control for one verb is the rule.
- **Editor.** On the dialog's surface, reading its ground, inks, accent and
  hairlines from probes styled with the dialog's roles, as the open
  message's body does, at the column's edge, Barlow 15/24 with paragraphs 12
  apart: what is written looks as app colours will draw it. It fills the
  dialog's remaining height.
- **No footer.** Attach and Remind are verbs in the action row; the word
  count is in the subtitle.

| State | What the composer shows |
|---|---|
| Empty (new message) | "New message", "Plain text · 0 words"; the caret in To; an empty editor on the dialog's surface. Send is live: sending with no recipient is refused with its reason |
| Reply, reply all, forward | The title names it; recipients as chips; "Re:" on the subject; the thread's labels with "from the thread"; the caret above the quote, folded to one "▸ Quoted message" line in muted ink on a hairline rule. The subtitle counts what will be sent, the quote included |
| Attachments | "Attachments" under the editor, in the column, then each file's name, its size and its remove button (shot 32) |
| Invalid recipient | The warning line under the editor ("Grac in To does not look like an address"); Send stays live and asks before sending |
| Sending | Not a state of the dialog: Send writes the Outbox and closes it at once (local-first); the list's toast says what happened |
| Offline | The same composer: Send queues in the Outbox (FR-055) and the window's banner says it is offline. Nothing greys out or waits |
| A send that failed, reopened | The subtitle says "Not sent — {reason}" in place of the saved time |
| Narrow (1024 × 768) | 655 × 688; the column stays 480; the four verbs fit the action row with their caps, a reminder's day included |

Compared at 1440 × 900 and 1024 × 768, light and dark (shots 05, 06 and
32): the dialog does not dim the list in the shots (libadwaita's dimming,
under the shot's animations off), and Detach is the icon theme's
`window-new-symbolic`, which at 16 px reads as a corner mark.

Pinned by `compose_layout` (the header, Send the one primary, short caps,
shared edges, the editor's surface, the shared X, the size rule), with
`postio-ui`'s `hints` cases.

## Reading beside the list

**The pane is the message dialog, placed beside the list.** It is one
message view, `open::OpenMessage`, moved between a dialog and a pane, so the
two cannot drift: the same header, action row, column, rhythm, components,
treatments and render-mode line. Only where it is drawn changes, never what
a key does.

**Switching.** One registry command, `toggle_reading_pane`, "Read beside the
list or over it", on `F8` (the key other mail clients give their message
pane), in the List context, which the Reader context falls back to, so it
works with a message open too. Focus only. The setting behind it is
`[focus] reading = "dialog" | "pane"` (contracts/config.md), default
`"dialog"`. The command switches the window at once and writes the setting
through the settings' own path (`toml_edit`, then `write_atomically`); the
watcher's echo of that write changes nothing. A message that is open moves
with the switch and stays open. A toast says which way messages now open,
and, when the window is too narrow for a pane, that they open beside the
list once it is wider. The main menu carries it as a check item, "Read
beside the list". **The dialog is the default:** it gives the message the
window's centre while the list stays in sight behind it; the pane is for
people who want both at once.

**Layout.** Under the top bar, the strip and the banner, which still span
the window: the list on the left, the pane on the right, a strong hairline
between them. The bulk bar stays at the window's foot, under both. The
geometry is `postio_ui::focus_dialog::pane_width`:

- The pane is `min(820, W - 404)`: 820 is the dialog at its widest, so a
  message reads at the same measure in both places, and 404 is the list's
  floor.
- The list takes the rest: 404 at 1024, 460 at 1280, 620 at 1440, 1100 at
  1920.
- The pane is never narrower than 576, the app colours column and its inset,
  so a pane needs a window at least 980 px wide (`PANE_WINDOW_MIN`). Below
  that a message opens in the dialog and the setting stays as it is.
  Crossing the line with a message open moves it between pane and dialog,
  still open, at its place in the thread.
- There is no draggable divider: the pane's width comes from the window, as
  the dialog's does, so it can never be dragged narrower than its column or
  wider than its measure.

Inside the pane, everything follows from its width as it does from the
dialog's: the column is `column_width(pane, treatment)`, and below 760 the
action row folds into More.

**Behaviour.**

| Input | With nothing open | With a message open |
|---|---|---|
| `Return`, double-click | opens the cursor's row in the pane | opens the cursor's row |
| `j`/`k`, the header's steps | move the cursor only (FR-016) | move the cursor, and the pane shows its row |
| a click on a row | moves the cursor only | moves the cursor, and the pane follows it |
| `Escape`, the pane's X | clears the selection | closes find first, then the message: the pane is empty, and the keyboard is in the list, on the row it was on |
| `a`, `Delete` | act on the cursor's row | act on the open message, and the pane steps past it to the next row, else the previous one, else it is empty |
| arrows, Page Up/Down, space, Home/End | move in the list | read the message, as in the dialog |
| `[`/`]`, `O`, `.`, `-`, `v`, `o`, `e`/`E`/`f` | the list's | the open message's, as in the dialog |

The list keeps its scroll, its cursor and the keyboard throughout: keys go
to the open message's commands because a message is open (`Context::Reader`),
not because the pane has the focus. **The pane follows the cursor only while
a message is open, and only `Return` opens one**, so `j`/`k` over the list
stays triage: no body is read for a row passed over, and no remote image is
fetched for a message nobody opened.

**The composer takes over the pane.** Reply, Reply all, Forward and `c` put
the composer where the message was, computed from the pane's width, with the
list beside it. Send or `Escape` hands the pane back to the message it
answered, or to the empty pane. Below 980 the composer has its dialog.

| State | The pane |
|---|---|
| Nothing open | "No message open", "Messages open here, beside the list.", and two shortcuts that run their commands and wear their keys: `↵` open, `F8` read over the list (`focus_state::empty_pane`, the empty inbox's pattern without its tray). Never blank, and never a dead end |
| The inbox is empty | No pane: the empty inbox takes the window, as in dialog mode. The pane comes back with the first row |
| Loading, partial body | The header, subject, labels and position are drawn from the row at once; the body says what the dialog's says when the body has not synced, never a spinner over local data |
| Offline | The window's banner says so. A body on disk reads as normal; one that is not says it is not downloaded yet. Every verb works and is queued |
| Failing | The banner names the reason and its key, and the pane is unchanged |
| Dense | A row narrower than the inbox's gives up its sender's column first, from 222 to no less than 120, so the subject keeps 260 for itself, its pills and the time (`focus_row::row_columns`); a marked row's second line then starts under the sender |
| Narrow (< 980) | No pane: the dialog |

The pane is the inbox list's. Filtered, the digest rules and a digest have
no list beside a pane, so a message opened from them opens in the dialog.

Compared (shots 34, 35 and 36, at 1440x900 and 1024x768, light and dark): at
1440 the list is 620 and the pane 820, the message exactly as 04's dialog
draws it; at 1024 the list is 404 and the pane 620, the action row folded
into More, the column still 480. Nothing behind is dimmed, since nothing is
behind, and the bulk bar stays at the window's foot.

Pinned by `reading_pane` (F8 and the setting, Return, `j`/`k`, Escape and
the X, a click, archiving, a window under 980, the column, the composer, the
menu's check item), with `postio-ui`'s `focus_dialog` pane cases,
`focus_state::empty_pane` and `focus_row::row_columns`, and `postio-config`'s
`set_reading`.

## Reading marks it read

Unread is only worth anything while it means "you have not looked at this",
and a rule that marks too eagerly destroys it the first time somebody walks a
mailbox.

**A message counts as read once it has stayed open for the dwell**
(`postio_ui::dwell::DWELL_TO_READ`, one second), in the dialog or the pane.
The clock starts when a message is shown and is cancelled when another is
shown, when the message closes, and when `r` is pressed for it. When it runs
out, Focus sends `MarkReadOnDwell` for the message the clock was started
for: the store is written, the row repaints without its bold.

- **Not at open.** `j`/`k` step the pane and `[`/`]` step the thread;
  marking at open would mark everything a person stepped past. A held `j`
  rests nowhere, so it marks nothing.
- **Only while the window is the active one.** A message left open in a
  window that has lost the focus is not being read: the clock stops when the
  window stops being active, and nothing is marked however long it stays
  away. When the window is active again the clock starts over for the same
  message, if it is still open and the clock had not already finished with
  it (marked, or kept unread by `r`).
- **The cursor marks nothing** (FR-016): with nothing open it starts no
  clock.
- **One message at a time.** The clock marks the message on screen, not the
  conversation.
- **The body does not hold the clock.** The header, subject and sender are on
  screen at once from the row.
- **The way back is `r`, not undo.** `MarkReadOnDwell` is kept off the undo
  stack: reading a mailbox makes one per message, and recording them would
  bury the action `mod+z` should bring back. `r` in the open message marks
  it unread, and cancels a clock still running, so the person's choice wins.
  `r` itself is undoable.
- **Local-first.** The mark is a store write and an enqueued flag change.

Pinned by `read_on_dwell` (left open in the dialog, stepped past in the
pane, closed before the dwell, `r` again, an unfocused window, refocusing), with `postio-ui`'s `dwell` cases.

## Sending states

ADR 0021 Decision 3. A draft that left the composer is in one of a few
states a person may have to act on, and Focus says which and offers what
settles it.

**Where they are listed.** Drafts lists what is being written, what stopped
("Not sent") and what nobody could confirm ("Not confirmed"). The Outbox
lists what is on its way ("Waiting to send", "Sending"). The Outbox is a view
over the Drafts folder, so `g o` lists it only while something waits in it,
under Drafts, with its count.

**The row.** The state's word (`postio_ui::row::send_state_word`) is drawn
in the trailing column, before the date: dim while the draft is on its way,
in ink once it needs the person. A draft being written says nothing more.
Screen readers hear the same word.

**Opening.** `Return` on a draft being written opens the composer. A draft in
any other state opens as the open message, in the dialog or the pane
(`focus_dialog::opens_to_read`), since opening is the only way to look and
the composer would change the draft. The subtitle carries the state's word.

**The action row** gives way to the verbs that settle the state
(`focus_dialog::send_verbs`), the registry's commands with their own keys:

| State | Offered |
|---|---|
| Waiting to send | Cancel send, Edit |
| Sending | nothing: a cancel is refused once submission starts, and a retry risks a second copy |
| Not sent | Retry send, Edit |
| Not confirmed | Retry send, Mark as sent, Edit |

Edit is `Return`: it closes the message and opens the draft in the composer,
taking a waiting send off the queue before anything is edited. Cancel send,
Retry send and Mark as sent act on the draft behind the open message, which
moves it to the other list, so the message closes and the cursor stays. On
mail that is not being sent, their keys say "That message is not one being
sent".

Pinned by `sending_states` (unconfirmed, stopped, edit on a queued send, the
pane), with `postio-ui`'s `focus_dialog` cases.

## The row menu

A right-click on a list row opens a menu of the row's verbs, the same verbs
the open message's action row and the bulk bar hold:

| Group | Verbs (each with its key from the keymap) |
|---|---|
| Open | Open (`Return`) |
| Answer | Reply, Reply all, Forward |
| Triage | Archive, Snooze…, Unsnooze, Remind if no reply…, Mark read / Mark unread, Flag / Unflag |
| File | Label…, Move…, Digest mail like this… |
| Lose | Delete, last and apart |

- **One command each.** A verb hands its `CommandId` to the window's one
  `act`, the path its key takes; the menu implements nothing. An ellipsis
  marks a verb a picker follows. The read and flag verbs say which way they
  go for the one row; Unsnooze shows only in the Snoozed list.
- **The cursor goes to the row**, so a picker the verb opens hangs from it.
- **Inside the selection** the menu is for the selection: its heading says
  "3 selected", and Open and the replies are left out. **Outside it** the
  menu is for the row alone, and the selection is let go only when a verb
  runs, so a stray right-click loses nothing.
- **Keys.** While it is up the menu has the keyboard: Escape closes it, a
  verb's own key runs that verb, the arrows and Enter walk and press its
  items. It does not grab, and a press outside closes it. There is no key
  that opens it: every verb in it already has its own.

Pinned by `row_menu` (its verbs and keys, a verb running its command, inside
and outside the selection).

## Settings

**A dialog over the list.** Settings is an `AdwDialog` over the window, as
every other surface Focus opens over the list is, and closes back to the
list's cursor, selection and scroll as they were. Not a window of its own (a
second toplevel goes behind its window and needs its own scheme and keys),
and not a page that takes the window (Settings is not mail to walk with the
list's keys).

**The message dialog's frame.** Its size is the open message's rule. The
header is the message dialog's: 52 px, "Settings" centred, the one X at the
right. Where the open message has its steps, Settings has its find-a-setting
field, which filters the section list. There is no action row: a section's
verbs (Add account, Reset to defaults, Revert file) sit in its pane, and the
file's are on the foot strip, where "Open in $EDITOR" carries its key
(`mod+e`). Every icon button is `icon_button()`.

**The panel.** Below the header is the shared settings window,
`postio_widgets::settings::SettingsPanel` (ADR 0031): the section list down
the left (214 px), one pane, and the foot strip (the file's state, the table
the pane writes, and the way out to the editor). Focus draws it in its own
type (C25) and accent (C26), which here paints only the keyboard focus ring:
the current section, a checked box and a key being rebound are drawn in the
neutral selection and the ink.

**What Focus shows.** Accounts, Filters, Composing, Keyboard, Sync &
storage, Privacy and Config file. Appearance is not shown: every key it sets
(`[ui]` theme, density, hover actions, avatars) is one Focus does not honour.
Keyboard lists the commands Focus offers, with their keys. Sync & storage
has "Back up locally": a check per folder, under its account's address when
there is more than one, cleared to skip that folder's backfill (ADR 0016).

**One column.** The dialog is at most 820 px wide, so a pane is at most 606:
a pane's two columns stack into one, the rule between them running across,
and the dialog is as wide as the pane on screen needs.

**Keys.** `mod+comma` and the main menu's Settings open it; `mod+comma`
again, Escape and the X close it. While it is open its controls have the
keyboard, and only the window's keys that mean something here are the
window's: Escape, `mod+comma`, and `mod+e`, which opens `config.toml` in
`$VISUAL` or `$EDITOR` from anywhere in Focus. With the keyboard on an
account row, `Return` (enable or disable), `Delete` (remove, undone by
`mod+z`), `r` (rebuild the index), `m` (make default) and `M` (map mailbox
roles) act on that row, and the command bar lists them. A Keyboard row
waiting for a key takes every key, Escape included, until it has one.
Changes write `config.toml` as they are made, and Focus follows the file
live (`Session::follow_config`): `[keys]`, `[saved_searches]`, `[sync]`, `[focus]`,
`[compose]`, `[reader]` and `[storage]` take effect without a restart.

**States.** Accounts with none says so and offers Add account; a folder list
appears once an account has synced its folders; the connection test says
what each server answered, in its words. Nothing in Settings touches the
network except "Test connection", when pressed.

Pinned by `settings` (`mod+comma` and the menu, every section and not
Appearance, Escape and the X, a privacy toggle and a signature persisting)
and `settings_wiring`.

## Rendering them

`cargo run -p postio-gtk --example shot -- <png> <screen> [light|dark] [WxH]`
writes one screen; an unknown screen writes nothing and says `NO IMAGE WAS
WRITTEN`. The cargo runner sends it to the private headless compositor, whose
1280x800 monitor mutter will not open a 1440x900 window on without maximizing
it, so the references' size wants a larger monitor of its own:

```sh
POSTIO_TEST_DISPLAY=focus-shot POSTIO_TEST_GEOMETRY=1920x1200 \
    cargo run -p postio-gtk --example shot -- /tmp/01.png 01
```

The shot runs with an empty `XDG_CONFIG_HOME` (a desktop's own `gtk.css` would
otherwise paint the picture) and with GTK's animations off (a capture taken
as the state is reached would otherwise catch the focus ring and the toast on
their way in).
