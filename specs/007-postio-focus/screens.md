# Screens: each build compared with its PNG

Spec FR-095 and SC-009. Each screen from 01 to 20 is rendered from the demo
store (`cargo run -p postio-focus --example shot -- <png> <screen>`). The
image is read back beside its reference in the maintainer's
`Design/postio-focus-design/screens/`, in light and dark. Every difference is
written down here with its reason. The references are never committed; they
carry a real name until they are re-rendered.

The differences already known before any screen was built come from the
spec's table *Where the inputs disagree* (C1–C23), and they are filled in
below. A phase that builds a screen adds what its comparison finds, and sets
the date.

| Screen | Compared on | Differences | Reason |
|---|---|---|---|
| 01 Inbox, light | 2026-09-28 | Known: no digest row (C5), no Task buttons or "Task in … · due" (C9), no filtered or digest counts (C10), shifted keys (C22). Found: (a) the sync label's icon is a bare check, not a check in a circle; (b) the field's keycap reads `ctrl+k`, not "Ctrl K", and the bulk bar's keys are one mono line ("x toggle · J/K extend · Escape clear"), not caps; (c) a selected row shows the icon theme's check glyph, not a filled box; (d) the bulk bar's buttons are bordered, not flat labels with Archive raised; (e) the cursor is the accent outline alone, with no accent tint; (f) the date, the counts ("58 · 19 unread") and "Has action · 8" are the demo store's. Fixed on the way: the list opened 33 px down with "Today" hidden (GTK anchors the first row, not its heading), and the strip painted no plate of its own Invite marker (T114, compared 2026-09-28): the anatomy is as drawn -- the "Invite" chip outlined in the accent, the date and times in the accent and bold ("Thu 1 Oct · 10:00–10:45", the demo's), and Accept `y` / Decline `Y` on the right; (g) Decline is a bordered button like Accept, where the reference draws Decline flat; (h) an answered, cancelled or past invitation, which no reference draws, shows "Accepted"/"Declined", "Cancelled" or "Past" in dim ink where the buttons were, and a click on Accept or Decline answers as `y`/`Y` do Question and To-do markers (T120, compared 2026-09-28): the anatomy is as drawn -- the chip outlined in the accent, a to-do's due day bold in the accent, the sentence verbatim in italics between curly quotes, and Reply `e` on a question; (i) a to-do offers Snooze `s` alone, with no Task `t`; (j) the due days are the demo's, set so many days from the day it runs, so "Fri 2 Oct" stands beside "by Wednesday" where the reference reads Wed 30 Sep, and the sentences are the demo's own ("… by Monday.", not "… by Monday, 28 September."); (k) the chip, the day and the quote are in the system accent (teal on the headless compositor), not the reference's blue; (l) Snooze is a bordered button, where the reference draws it flat beside a bordered Task | Milestone order (C5, C9, C10); (a) the icon theme's `emblem-ok-symbolic`; (b) and C22: the shared hint code's spelling and the shared `KeyLine`; (c) the stock `checkbox-checked-symbolic`; (d) the shared `ActionBar`'s buttons; (e) FR-091: the accent is the focus ring, and contracts/focus-surface.md draws the cursor as a 2 px ring that tints nothing; (f) the demo is anchored at 16:09 on the day it runs; (g) the row draws every answering action as one button kind; (h) FR-103, through `postio_ui::focus_row::marker_line`, and constitution II: every key has a visible button; (i) C9: Task joins Snooze on a to-do only when `[focus.vault]` is configured (T158); (j) the demo seeds its markers rather than running the detector, which reads "by Wednesday" against the message's own date (US12 scenario 2, `postio-classify`); (k) FR-091: a marker is the one thing on a row in the accent, read from `AdwStyleManager`; (l) as (g) |
| 02 Inbox, dark | 2026-09-28 | As 01. Libadwaita's own dark palette, with nothing of Focus's in between: the neutral selection and the hairlines lift rather than darken, and the markers, the unread dots and the cursor ring take the dark accent | As 01; FR-090 |
| 03 Has-action filter | 2026-09-28 | As 01 (C9, C22, and a–f). The reference heads its list "Has action · 7" and draws eight rows; the build's heading counts the rows it lists ("Has action · 8" over eight) The Invite marker as 01 (g, h) Question and To-do markers (T120) as 01 (i–l); the reference's "Sun 27 Sep" beside "by Sunday night" is the demo's "Tue 29 Sep" (j) | As 01; the heading and the toggle read one count (`FocusCounts.has_action`); As 01; As 01 (i–l) |
| 04 Open email | 2026-09-29 | Known: the body is drawn by the new renderer from the part the reader draws (C1), and Task and Note wait for Obsidian (C9). The dialog is one scrolling column (T183): the thread chip, subject, labels, header card, marker card, body, attachments and fold line scroll together, and the body is drawn flat on the dialog's white at 15px, with no frame, filling the column, which runs the dialog's width less a `--postio-space-6` gutter a side (T197: the reference's 860px column and 700px measure left a dead margin beside the text, and the maintainer found the sides too padded); a message that paints a page of its own (a sender's sheet, a page background) keeps a quiet hairline frame. The header card is a tinted card (T184): small dim From, To and Cc labels, the sender's name bold and the address dim mono, a Cc line only when someone was copied, and "Today, 15:22" for today's mail, the absolute date beyond. Attachments are cards under the body (T185): icon, the name in mono, the size beneath. The label pill carries its colour dot (T186). Close is the toolbar's compact 26px pill (T187). Found: (a) a plain-text body kept its blank lines as full lines, a paragraph break a line and a half wider than the reference's 12px gap (fixed by T203: a 12px gap); (b) the toolbar's buttons are the shared action bar's, a quiet 26px kind, without the bold words; (c) the fold line says "3 quoted lines folded", not "… from v2 folded": the fold knows its line count, not which message it quotes; (d) keycaps are spelled as the keymap spells them, `Escape` rather than "Esc" (C22); (e) the thread chip's `[` is a cap, and the steps are icon buttons with their caps beside them; (f) the body's face is the bundled Barlow, the reference's is the system's. The marked sentence is highlighted where the body draws it, with a line under it, as drawn Invite (T114): the reference opens a to-do, and its only Invite is the row behind the dialog, as 01. Opened, an invitation's card under the header reads "Invite", its date and times, and Accept `y` / Decline `Y`, which answer the invitation on screen; the toast then reads "Accepted" or "Declined" and stays for the answer's window | (a) the renderer's plain-text presentation (a `pre`); (b) the shared widgets as they are; (c) the fold line reads what the document knows; (d) C22; (f) ADR 0023's faces; the marker card is 04.5's anatomy for every kind; FR-102 |
| 05 Compose | 2026-09-28 | Known: no Markdown toggle (C7); suggestions open at four characters, "Grac", not three (C23); Cc and Bcc share one control, the composer's "+ Cc", not "Cc Ctrl ⇧C · Bcc Ctrl ⇧B"; no "Task after sending" (C9); keys spelled as the keymap spells them, `Escape`, `ctrl+Return`, `ctrl+shift+a` (C22). Found: (a) the suggestions popover is not in the picture; (b) a suggestion row reads "Name <address>" with no "wrote 42 times", "list" or footer; (c) the formatting toolbar sits between the fields and the body; (d) From comes after To (and Cc), not first, and reads "Name <address>" with no ▾ for one identity; (e) the footer's "Remind if no reply" (`mod+h`) came later with T096 and now matches; (f) no "Draft saved locally" under the heading until the first save lands; (g) the half-typed "Grac" earns "Grac in To does not look like an address" above the footer; (h) Send is a raised neutral button, not a dark fill; (i) a detach button sits before Send later; (j) the Labels row is empty and has no way to add one; (k) the list behind is not dimmed | C7, C9, C23, C22; (a) a popover is a surface of its own, and the window capture draws the window's; (b) a `RecipientCandidate` carries only the address, not its sent count, so the count needs a model change (reported); (c)(d) the existing composer's own layout (FR-050), which Focus frames rather than rebuilds; (e) built in T096; (f) the subtitle says a save that happened, never one that has not; (g) the composer's recipient warning, shared with the classic app; (h) contracts/focus-surface.md: raised buttons are plain raised buttons with bold labels, and the accent is reserved (FR-091); (i) FR-050: the composer may be detached, and its own control for it is in the row the frame hides; (j) labels are added with the label picker (T097); (k) libadwaita's dialog dimming, under the shot's animations off |
| 06 Reply all | 2026-09-28 | As 05 (C7, C9, C22, and c, d, e, f, h, i, k). Found: (a) the quote is the composer's fold, "On 2026-09-28, Juno Castellane wrote:" over a closed "Quoted message", not "› On Sat 26 Sep, 15:22, Lena Park wrote · 18 quoted lines Ctrl ⇧Q show"; (b) the footer reads "Rich text · 24 words", counting the quote; (c) "1 To, 3 Cc" sits above the footer; (d) the body shows no reply typed above the quote; (e) the names are the demo store's | (a) the fold is the shared editor's (`postio_ui::editor::document::fold_quotes`), opened by a click on its summary; the one keymap has no command for Ctrl ⇧Q, and adding one is a keymap-contract change (reported); (b) the quote is part of what is sent, as rich text; (c) the composer's recipient count, shared with the classic app (FR-023); (d) the shot does not type into a reply, which would replace the quote; (e) the demo is anchored to the day it runs |
| 07 Search, plain English | 2026-09-28 | As 01 (b–f). Typed as "the invoice Marisol sent last month", the demo's correspondent. Known: "invoice" stays a free word, so its chip reads `invoice`, not `subject: invoice`. Found: (a) chips are plain mono boxes with no × and no editing, and Tab does not move into them; there is no "editing after:", no Ctrl ⌫ "back to plain words" and no Ctrl S (the reference's corner note); (b) the saved searches carry no counts; (c) the "Search mail for …" row stays above the results, and the count sits in the heading ("Conversations · 3 matches") rather than at the right; (d) the bar lies 8 px under the strip, over the list, not 60 px from the window's top over the strip too; (e) the footer is one line of text, not caps. Fixed on the way: "Ada"/"Marisol" was a free word because the bar handed `natural::lower` an empty address book (now the account's correspondents); the list did not dim | Research R5; (a), (b) not built yet (T086's chip editing and save; saved-search counts are a count per query); (c) one results list for every kind of row; (d) the bar is an overlay on the list's area; (e) the shared hint spelling (C22) |
| 08 Go to a folder | 2026-09-28 | As 07 (a, b, d, e). `in:Rec` stays typed in the entry rather than becoming an `in: Receipts` chip. The rows have no address column, and the first column is the sender's name as the demo files it ("Receipts") | The bar completes `in:` to a folder and lists it, but does not rewrite what was typed (T086) |
| 09 Commands | 2026-09-28 | Known: "Archive everything read, older than a week" has a key (C11). Found: (a) the command rows are the palette's titles with no detail ("the focused message · Re: …", "3 messages") and no bold on the matched letters; (b) the blend also matches "arch" inside "Search" and "Saved search 1", so those rows are listed where the reference has "Undo: unarchive last"; (c) `in:Archive` has no conversation count; (d) the search row sits under "Go to" with no "Search" heading and no ⇧↵ cap. Fixed on the way: "arch" showed twice, in the entry and as a chip | Constitution II; (a), (b) and (c) `postio_ui::finder::blend` and the palette as they are, shared with the classic app; (d) one list with one heading per kind that has more than one row |
| 10 Folders and labels | 2026-09-28 | As 01 (b, f). The demo store has no Snoozed or Filtered mailbox and fewer folders; it has Junk and Trash, which the reference leaves out. Labels have no counts. The Inbox mark is the icon theme's envelope (Adwaita has no tray), and Archive's a folder. No row is ringed on open; the first place is selected. Fixed on the way: the picture had no popover (a popover is a surface of its own, and the capture drew the window alone); GTK centred the popover on "Inbox", out past the window's left edge; the section heading was lit with the first row; the rows had no marks | The demo store; a label count is a query per label, not read yet; the icon theme |
| 11 Snooze picker | 2026-09-28 | As 01 (b–f). C14 settled: both pickers say "Later today", and "Tomorrow evening" once 6pm has passed. Found: (a) the title row names "3 conversations" where the reference names one conversation over the same three-row selection; (b) the number keys are framed caps, taller than the reference's bare digits, and the selected preset is tinted rather than ringed; (c) the times follow the demo's real clock, so on a Monday "Monday morning" and "Next week" name the same day; (d) the footnote's key is the keymap's spelling. Fixed on the way: a picker opened by a key was dismissed at once by the compositor (it no longer grabs; a press outside closes it) | (a) US5: a picker acts on the selection when there is one, and says so; (b) the shared cap (C22) and the list's selection; (c) the demo is anchored to today; (d) C22 |
| 12 Remind picker | 2026-09-28 | As 11 (a–d). The placeholder reads "tue 9am" in both pickers, where 12 reads "thu 2pm"; the footnote's "No reply since" is dated today | One placeholder for the one date field |
| 13 Label picker | 2026-09-28 | As 11 (a, b). With three conversations selected that share no label, no row says "✓ applied" (the reference shows Atlas applied to one). Counts are conversations. Fixed on the way: rows that landed after the popover opened scrolled inside its empty height; it is presented again when they land | "Applied" means on every conversation the picker acts on |
| 14 Move picker | 2026-09-28 | As 11 (a, b). The demo has one recent destination and two folders, so Recent has one row and All folders two; the footnote spells the keys as the keymap does ("Return", "ctrl+z"). Fixed on the way: a recent folder was listed again under All folders | The demo store; C22 |
| 15 Undo toast | 2026-09-28 | Shifted keys (C22). The toast says "Archived 4 messages", not 3: the three selected conversations hold four messages. Its Undo button carries no keycap, and the toast has a close button | C22; the engine counts the messages an action moved; AdwToast's button takes a label only, and its close button is libadwaita's |
| 16 Empty inbox | 2026-09-28 | Lists only what exists (C10), and the strip's own "186 filtered today · 4 digest rules" waits with it. The tray is the icon theme's folder: Adwaita has no inbox icon, so the themed fallback shows. "Inbox is empty" is the reading size, smaller than the reference's. The shortcuts are the shared ghost buttons, their labels bold where the reference's are regular. The sync label is the demo's "Synced 16:09" Compared again for T141: the strip says "186 filtered today g f · 1 digest rule g d", and "Next digest: Weekly · Newsletters, Saturday 16:00" is drawn from the demo's rule, as the reference has them; there is no digest row on 16, and the reference has none | C10; the icon theme; the shared button kinds |
| 17 First sync | 2026-09-28 | As 01 (b–f). The progress bar runs the banner's full width under it, where the reference draws a short bar after the sentence. The label reads "Syncing 12,408 of 18,204", as drawn | An AdwBanner holds a title and a button, nothing between; the bar is re-dressed neutral, not the accent's fill (FR-091) |
| 18 Offline | 2026-09-28 | As 01 (b–f). "Retry now" sits at the banner's right edge, where the reference sets it beside the sentence | AdwBanner places its button at the end |
| 19 Sign-in error | 2026-09-28 | As 18. The server and the address are the demo account's. "Update password…" does not open anything yet | As 18; the credential dialog is still `postio-gtk`'s (T055) |
| 20 Key map | 2026-09-28 | Known: the footer names `[keys]` in `config.toml` (C3), no Obsidian group (C9), keys spelled as the keymap spells them, `ctrl+k` rather than "Ctrl K" (C22). Found: one row per command, each with every binding, alternates included, where the reference merges pairs ("Top / bottom", "Extend selection") and shows one or two keys; and the groups hold every command Focus offers in the key map's contexts, zoom, find in message, the outbox's verbs and the picker's number keys among them, where the reference shows a shorter set. So the columns run past the dialog and scroll | Constitution II: the key map is generated from the registry and the groups table (`postio_ui::keymap_sheet`), and a command Focus offers is taught; merging pairs, or leaving a row out, is a change to that table rather than to the dialog |
| 21 Filtered | 2026-09-28 | Known: C4's copy, "Nothing here is deleted automatically · nothing here ever reached the inbox", where 21 says "Kept for 30 days, then deleted". Found: (a) the header's right holds "Sweep the inbox… F", where 21 has search and close: the sweep (FR-118) needs a control, and this is where filtering lives; (b) the day heading counts the rows of that day read so far, which is the tab's count until more than a page is filtered in a day; (c) the footer's keys are the keymap's spellings, `Return` for ↵ (C22) -- `Context::Filtered` now binds j/k, Return and `g i` as the footer promises; (d) the footer is one mono line, not caps; (e) the focused row is the accent ring, as the list's cursor is. The demo files nine, not 186 | C4 (maintainer); (a) FR-118; (b) the view reads fifty rows at a time; (c) C22; (d) and (e) as 01 |
| 22 Digest summary | 2026-09-28 | Rendered on a two-statement summary over the demo's six newsletters (`shot … 22`): "Rates" citing the rate decision, "Engineering reading" citing the CRDT comparison. Found: (a) the sub-row's tabs read "Summary \| 6 messages `Tab`", the demo's own count, not 14; (b) a statement is its own paragraph, one a line, with its reference as a plain trailing digit, where the reference draws one flowing paragraph a topic with boxed numbers inline after the sentence that earns them -- `gtk::Label` has no inline-widget text flow, and building one was out of this task's reach; (c) the focused reference's message card sits once, after every statement, rather than directly under the paragraph that cites it, and reads only the subject, with no border, chip or date; (d) no CSS exists yet for any `.focus-digest-summary-*` class, so nothing is bordered, coloured or ringed -- true of the plain list already; (e) the window has no footer line (`next/previous reference · open the referenced email · summary/messages · stop digesting the referenced sender`); the plain list draws none either | (a) the demo store; (b) and (c) a GTK label paragraph, not the render engine's rich text layout (documented here rather than built); (d) as the milestone-1 list; (e) as the milestone-1 list |
| 23 Email from a digest reference | 2026-09-28 | Opened from reference 1 of 22's summary (`shot … 23`), the passage highlighted through `TextIndex::locate`. Found: (a) the header reads "Summary `Esc`" and the title/subtitle as the reference draws them, but the toolbar (Reply, Forward, Archive, Note, Label, Unsubscribe, Stop digesting sender) is not drawn -- only the banner and the body; those verbs still reach the message through the plain list; (b) no "`j`/`k` next and previous source" hint is drawn in the header, though both keys work; (c) the banner has no left border or tint, for the same reason as 22 (d); (d) the body card is the shared reader, unstyled beyond it, as the open-message dialog's is | (a) and (b) scoped out of T154 for time -- the keys and the highlight are what US13's scenarios test, not this toolbar; (c) as 22 (d); (d) as the open-message dialog |
| 24 Digest this sender | 2026-09-28 | Known: "Match a list or a search instead…" was absent when first compared; T155 built it, and "Digest mail like this" appears only with a model configured. Found: (a) the demo's Oak Hill sender has one message in 90 days, so the preview lists one and says no "and N more"; (b) the key caps are the keymap's spellings, `Escape` and `Return` (C22); (c) Create is the shared primary button, bordered, where the reference's is filled black; (d) the cadence and day are GTK drop-downs; (e) the list behind keeps its cursor on the row whose sender is digested, with no selection: a selection would make the rule for every selected sender ("Digest these senders"), where the reference names one sender over three selected rows | (a) the demo store; (b) C22; (c) the shared button kinds (ADR 0043); (d) the toolkit's menus; (e) US10: "Digest these…" is one rule for the selection's senders |
| 25 Obsidian capture | 2026-09-28 | Rendered over a throwaway vault of three projects, on the demo's to-do "Please leave comments by Wednesday" (`shot … 25`, the project list opened as drawn). Known: the preview puts the link before the date, `… [✉](postio://message/84) 📅 2026-10-02`, where the reference ends the line in the link (C21); keys are spelled as the keymap spells them, `Escape`, `ctrl+Return`, `ctrl+p`, `alt+s` (C22). Found: (a) Task and Note are a linked pair of toggle buttons, the chosen one grey, not a white segment on a grey track; (b) Add task is the shared primary button, raised and bold, not a dark fill; (c) the text is a plain entry, with no accent-ringed card round the Task field; (d) "Due" has no "from “by Wednesday”"; (e) the quick picks are Today and the coming Monday, Wednesday and Friday in date order, then None -- on the demo's Monday, "Today, Wed, Fri, Mon, None" -- the chosen day ringed as drawn; (f) the suggestion reads "the subject names Harbor", not "Lena Park is linked from 23 notes in Harbor"; (g) the project line gives the note, with no "› ## Inbox" heading; (h) the Inbox row reads "Tasks.md (no project)", projects are listed by name, and "N open" counts only the tasks Postio captured; (i) the preview wraps at the dialog's width, and the footnote is under the fold at this height; (j) the footnote says where the task goes, not "The row will then show “Task in Harbor · due Wed”"; (k) the dialog is 660 px wide and dims the list, whose to-do rows show Task `t` because a vault is configured | C21, C22; (a) and (b) the shared toggle and button kinds, and FR-091 reserves the accent and a dark fill for nothing; (c) the entry is GTK's own; (d) a marker keeps the day it read, not the words it read it from (data-model.md, markers); (e) quick picks are relative to the day the sheet opens, and the mail's own day is always among them; (f) `postio-vault` suggests from the subject's words alone (`Reason::NamedInSubject`), with no link graph read; (g) FR-180: a capture is appended to the end of the note, never under a heading; (h) `Vault::tasks` reads back only lines with a `postio://` link; (i) the dialog scrolls rather than grow past the window; (j) the row's "Task in … · due …" chip is not built yet (FR-181's read-back is wired, the row does not draw it); (k) FR-092, one dialog pattern for every window over the app, and C9 |

## Interaction rules (T192-T194)

**One close rule (T192).** Every surface that closes (the window's top bar,
the open message, the composer, the digest, the raw source, the key map) wears
the one X icon button `postio_widgets::widgets::close_button()` builds, at the
right end of its header, after that header's verbs, with no keycap (Escape
closes and the key map says so). A surface with a step back (the digest's
email page) has that at the left, worded "Summary". No surface draws its own
close; `close_buttons::every_closable_surface_has_the_same_x_at_the_right`
walks them and asserts the widget and that nothing in the header is right of it.

**One icon-button rule (T202).** Every button in Focus that shows an icon
and no words is the one `postio_widgets::widgets::icon_button()` builds (or
`icon_menu_button()` for one that opens a menu, or `dress_icon()` for a
button whose icon is set elsewhere, the composer's formatting toolbar): the
ghost `.postio-icon-button`, 26px, its name as tooltip and accessible name,
and centred where it stands at its own size. The constructor sets the
centring, so no caller can forget it: left at GTK's default `Fill`, a 46px
bar or a wide column stretches the button and it hovers as a tall pill
(T193). `icon_buttons::every_surfaces_icon_buttons_keep_their_own_shape`
walks the window, the key map, the raw source, the open message and the
composer (and `…the_digests…` the digest), and asserts each icon button wears
the class and is allocated exactly what it asks for. Converted with it: the
main menu (now `icon_menu_button`), the composer's detach and label removes,
the recipient chips' removes, the formatting toolbar and the notice's "more"
menu, which each drew libadwaita's `flat` look at its own size.

## Reading the open message (T203)

Decided with `/gtk-design` against screen 04 and the shots, light and dark.

- **The measure.** The body is capped at `32em`, about 75 characters of
  Barlow at the 15px reading size (measured: 73-76 on a line of prose, where
  T197's full column ran 146). It keeps the column's left edge, under the
  toolbar's words, the subject and the header card, rather than centring:
  one left edge for the eye to return to, as the reference draws it (its
  720px measure sits left in its column), and the space beside a short
  measure reads as margin, where a centred block would put the text on a
  second edge of its own. The header card and the attachment cards still
  run the column's width: they are cards, not prose. `em`, not `ch`: a
  `ch` is the width of Barlow's `0`, wider than its average letter, so
  `75ch` would be nearer 95 characters.
- **The paragraph gap.** A plain-text body's blank lines part paragraphs
  (`postio_body::quote::text_to_html`, one `<pre>` a paragraph, however many
  blank lines stood between them) and the column draws the break as a
  `0.8em` gap, the reference's 12px, instead of an empty line the body's
  24px line-height tall. The classic reader gets the same paragraphs, with
  its own stylesheet's `1em` gap, where it drew a whole blank line.
- **The face: Barlow, kept.** The design system's fonts are Barlow, Barlow
  Condensed and IBM Plex Mono, and its rule is body copy in Barlow.
  Condensed letterforms tire a reader over paragraphs, which is why the
  system keeps them for headings; Plex Mono is the role for counts, keys and
  addresses, and an even-width face over prose reads as code. Barlow's
  low-contrast grotesque at 15px with a 1.6 line-height and a 75-character
  measure is a comfortable long-reading setting: what was tiring was the
  146-character line, not the face.
- **Find keeps the place.** `mod+f` in the open message opens find (it
  reached nothing before); the bar sits above the column, between the
  toolbar and the message, not at the column's top, so neither opening it
  nor its entry taking the keyboard scrolls the message; a query's first
  match is the first from the top of what is in view; `mod+g`/`mod+shift+g`
  step; Escape closes find before it closes the message.
- **The ground is a token.** The body was drawn on two hex values written
  into the reader's stylesheet, and dark's (`#1e1e1e`) was a shade off
  libadwaita's view colour the dialog is drawn on (29, 29, 32). The column
  now carries a probe (`.postio-flow-ground`) whose `color` is
  `--postio-surface`, the dialog's own role; the body view reads it at each
  render and hands it to the document as `--flow-ground`, and reads it again
  once a switch of scheme has restyled the window.

Pinned by `open_measure::the_measure_is_near_seventy_five_characters_from_the_left_edge`,
`…a_plain_paragraph_break_is_a_short_gap_not_a_blank_line`,
`…opening_find_keeps_the_reading_position` and
`…the_columns_ground_is_the_dialogs_own_in_light_and_dark` (the body's
pixels against the dialog's, in both schemes), with
`postio-body`'s `a_blank_line_parts_paragraphs_rather_than_drawing_an_empty_line`
and `postio-widgets`' `the_flow_sheet_names_its_ground_and_writes_no_colour`.

**Mouse and keyboard pairs (T194).** Each row is an action a person expects
from either device; "n/a" means the surface has no such thing.

| Surface | Keyboard | Mouse | State |
|---|---|---|---|
| List row | `j`/`k` move the cursor | click moves the cursor | present (GTK's single selection) |
| List row | Enter opens | double-click opens, through the same `OpenMessage` command | fixed (T194) |
| List row | `x` toggles selection | press in the row's gutter toggles it | fixed (T194) |
| List row | `Shift`+`j`/`k` extends | Ctrl-click toggles, Shift-click ranges, each through the commands the keys run (`ToggleSelection`; `ExtendSelectionDown`/`Up` from the anchor) | fixed (T198) |
| List row | the verbs' own keys | right-click menu of the row's verbs | fixed (T199) |
| List | `g g`, `G`, PageUp/PageDown | scroll wheel and scrollbar scroll | present (GTK's scrolled window) |
| Open message | Escape closes | the X closes | present |
| Open message | `j`/`k` step | the up/down buttons step | present (T195 owns the key) |
| Composer | Escape asks to close | the X closes | present |
| Key map, raw source, digest | Escape closes | the X closes | present |
| Pickers, command bar | Escape closes | a press outside closes | present |

## The composer (T221)

Decided with `/ux-architect` and `/gtk-design` from the maintainer's walk
(2026-10-01: "the compose window looks pretty bad ... especially the buttons
at the top"), against the message dialog it now matches (T205-T214). The
walk found the header carrying five controls and two long keycaps: the title
was centred but the verbs reached it from the right, `ctrl+shift+Return` was
the widest thing in the bar, Send's cap made a box inside a box, Attach was
drawn twice (the footer and the toolbar's paperclip), and the editor sat on
a grey of its own under rows on another.

**The rule: the composer is the message dialog with the composer's verbs.**
Pressing Reply in the message dialog puts Send where Reply was.

- **Size.** The message dialog's rule (`postio_ui::focus_dialog`, T205):
  `clamp(640, W - 2 * max(96, 0.18 * W), 820)` wide, the window less 80
  tall, following the window's resizes. 820 x 820 at 1440 x 900, 655 x 688
  at 1024 x 768. It was a fixed 980 x 820, wider than the window's own rule
  allowed at 1024.
- **Header bar, 52px: the message dialog's anatomy.** Left, Detach, an icon
  button (T202) where the message dialog has its steps; it carries its key in
  its tooltip, not a cap, because it is the rarest verb here. Centre, the
  title ("New message", "Reply", "Reply to all", "Forward") at the message
  dialog's title size, and under it, in the mono subtitle the message
  dialog uses for "Message 5 of 60", what will be sent and what has happened
  to it: "Plain text · 58 words · Draft saved locally 16:12". Right, the one
  shared close X (T192). Nothing else: no verb reaches the title from either
  side, at any width.
- **Action row, 44px between two hairlines: the composer's verbs.** Send
  `ctrl+↵`, Send later `ctrl+⇧+↵` with its ▾, Attach `ctrl+⇧+a`, Remind if no
  reply `ctrl+h` (and its day once chosen). Send first, where Reply sits in
  the message dialog's row. Send is the dialog's one primary button -- in
  Focus a plain raised button with a bold label (FR-091: no button wears
  the accent or `suggested-action`) -- and the others are the message
  dialog's quiet row verbs, no frame of their own and a tint under the
  pointer. Send later is secondary in the same way Reply all is beside
  Reply: the same kind as the other verbs, after Send.
- **Keycaps: the keymap's spelling, compacted.** `hints::short`, which the
  message dialog already uses for `Del`, now also draws `Return` as `↵` and
  `shift` as `⇧` -- the glyphs Focus's own hint lines (`↵ open`) and the
  classic rail (`⇧I`) already draw. `ctrl` stays a word, as the command bar's
  `ctrl+k` spells it, and the `+` stays. `ctrl+shift+Return` (17 characters)
  is `ctrl+⇧+↵` (8). What is pressed and what a screen reader hears keep the
  binding's own names. The caps are the message dialog's: 16px, an inset
  hairline, muted.
- **One column: the reader's.** Everything under the action row -- the field
  rows, the formatting toolbar, the editor, the recipient warning and the
  attachments -- shares one column, `min(480, dialog - 96)`, centred:
  the app colours column the message is read in (T207). So the left edge of
  To, of the toolbar and of the first line written is one edge, and a reply
  is written at the measure it will be read at, about 70 characters. The
  wider paper column is for mail that paints its own page; nothing written
  here does.
- **Field rows: the sender block's grid.** To (with "+ Cc" at its right),
  Cc and Bcc when shown, From, Subject and Labels: 40px rows, each with a
  hairline under it, the names in one label column as wide as the widest
  name (a size group, so "Subject" and "To" start their values at one x),
  in the message dialog's muted 13px, and the values in ink, 12px after it
  (the sender block's column gap). Every row runs the column's full width.
- **Formatting toolbar.** The composer's own icon buttons, the shared
  26px ghost (T202), in one row under the fields, its first button at the
  column's left edge. The paperclip is hidden in Focus: Attach is a verb in
  the action row with its key on it, and one control for one verb is the
  rule. The classic composer keeps it.
- **Editor: on the dialog's surface.** The editing document reads its
  ground, ink, secondary ink, muted ink, accent and hairlines from probes
  styled with the dialog's own roles, exactly as the open message's body
  does (T203, T211), and draws its text at the column's edge (no inset of
  its own), Barlow 15/24 with paragraphs 12 apart: what is written looks as
  the app colours treatment will draw it to be read. It fills the dialog's
  remaining height. The classic composer keeps its own ground and inset.
- **Footer: none.** Its three parts moved: Attach and Remind to the action
  row, where verbs are; the word count to the subtitle, where the message
  dialog says its counts. One band less, and the editor has its height.

**States.**

| State | What the composer shows |
|---|---|
| Empty (new message) | "New message", "Plain text · 0 words"; the caret in To; an empty editor on the dialog's surface. Send is live: sending with no recipient is refused by the composer with its reason, as before |
| Reply, reply all, forward | The title names it; recipients as chips; "Re:" on the subject; the thread's labels with "from the thread"; the caret above the quote, folded to one "▸ Quoted message" line in the dialog's muted ink on a hairline rule. The subtitle counts what will be sent, the quote included |
| Attachments | Cards under the editor, in the column, as the message dialog draws its attachments |
| Invalid recipient | The composer's warning line under the editor, in the column ("Grac in To does not look like an address"); Send stays live and asks before sending |
| Sending | Not a state of the dialog: Send writes the Outbox and closes it at once (local-first); the list's toast says what happened |
| Offline | The same composer: Send queues in the Outbox (FR-055) and the window's banner says it is offline. Nothing greys out or waits |
| A send that failed, reopened from Drafts | The subtitle says "Not sent — {reason}" in place of the saved time |
| Narrow (1024 x 768) | 655 x 688; the column stays 480; the four verbs fit the action row with their caps |

Pinned by `compose_layout::the_header_is_detach_title_close_and_the_verbs_have_a_row_of_their_own`,
`…send_is_the_one_primary_and_nothing_wears_the_accent`,
`…the_keycaps_are_short`, `…the_field_rows_share_their_edges`,
`…the_editor_is_drawn_on_the_dialogs_surface`,
`…the_close_is_the_shared_x` and `…the_dialog_follows_the_message_dialogs_size_rule`,
with `postio-ui`'s `hints` cases for the compact spelling.

## The row menu (T199)

A right-click on a list row opens a menu of the row's verbs, decided as
`/ux-architect` from the open message's toolbar and the bulk bar, which are
the two places those verbs already live:

| Group | Verbs (each with its key from the keymap) |
|---|---|
| Open | Open (`Return`) |
| Answer | Reply, Reply all, Forward |
| Triage | Archive, Snooze…, Remind if no reply…, Mark read / Mark unread |
| File | Label…, Move…, Digest mail like this… |
| Lose | Delete, last and apart |

- **One command each.** A verb hands its `CommandId` to the window's one
  `act`, the path its key takes; the menu implements nothing. Its words are
  the toolbar's and the bulk bar's, an ellipsis where a picker follows; the
  read verb says which way it goes for the one row. Flag is not offered:
  Focus answers no flag command yet.
- **The cursor goes to the row**, as a click's does, so a picker the verb
  opens hangs from that row.
- **Inside the selection** the menu is for the selection: its heading says
  "3 selected", and Open and the replies, which need one message, are left
  out. **Outside it** the menu is for the row alone, and the selection is let
  go only when a verb runs: Escape or a press outside leaves it as it was, so
  a stray right-click loses nothing. (The file managers' rule -- a
  right-click selects the row -- would drop a selection on every dismissed
  menu.)
- **Keys.** While it is up the menu has the keyboard: Escape closes it, a
  verb's own key runs that verb from it, the arrows and Enter walk and press
  its items. It does not grab (as the pickers do not, for the same reason),
  and a press outside closes it.
- **Not built:** opening the menu from the keyboard (`Menu`,
  `Shift+F10`). Every verb in it already has its key, so it teaches rather
  than gates; a binding for it would be a registry command, and the keymap
  contract is not this task's to change.
- The hook in the list is one secondary-button gesture on the list view
  (`ListPane::connect_row_menu`), clear of the row widget's own clicks.

Pinned by `row_menu::a_right_click_on_a_row_offers_its_verbs_with_their_keys`,
`…a_menu_verb_runs_its_command_on_the_row` and
`…a_right_click_outside_the_selection_is_for_that_row_inside_it_for_the_selection`.

## Rendering them

`cargo run -p postio-focus --example shot -- <png> <screen> [light|dark] [WxH]`
writes one screen; an unknown screen writes nothing and says `NO IMAGE WAS
WRITTEN`. The cargo runner sends it to the private headless compositor, whose
1280x800 monitor mutter will not open a 1440x900 window on without maximizing
it, so the references' size wants a larger monitor of its own:

```sh
POSTIO_TEST_DISPLAY=focus-shot POSTIO_TEST_GEOMETRY=1920x1200 \
    cargo run -p postio-focus --example shot -- /tmp/01.png 01
```

The shot runs with an empty `XDG_CONFIG_HOME` (a desktop's own `gtk.css` would
otherwise paint the picture) and with GTK's animations off (a capture taken
as the state is reached would otherwise catch the focus ring and the toast on
their way in).
