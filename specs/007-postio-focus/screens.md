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
| 01 Inbox, light | 2026-09-28 | Known: no digest row (C5), no Task buttons or "Task in … · due" (C9), no filtered or digest counts (C10), shifted keys (C22). Found: (a) the sync label's icon is a bare check, not a check in a circle; (b) the field's keycap reads `ctrl+k`, not "Ctrl K", and the bulk bar's keys are one mono line ("x toggle · J/K extend · Escape clear"), not caps; (c) a selected row shows the icon theme's check glyph, not a filled box; (d) the bulk bar's buttons are bordered, not flat labels with Archive raised; (e) the cursor is the accent outline alone, with no accent tint; (f) the date, the counts ("58 · 19 unread") and "Has action · 8" are the demo store's. Fixed on the way: the list opened 33 px down with "Today" hidden (GTK anchors the first row, not its heading), and the strip painted no plate of its own | Milestone order (C5, C9, C10); (a) the icon theme's `emblem-ok-symbolic`; (b) and C22: the shared hint code's spelling and the shared `KeyLine`; (c) the stock `checkbox-checked-symbolic`; (d) the shared `ActionBar`'s buttons; (e) FR-091: the accent is the focus ring, and contracts/focus-surface.md draws the cursor as a 2 px ring that tints nothing; (f) the demo is anchored at 16:09 on the day it runs |
| 02 Inbox, dark | 2026-09-28 | As 01. Libadwaita's own dark palette, with nothing of Focus's in between: the neutral selection and the hairlines lift rather than darken, and the markers, the unread dots and the cursor ring take the dark accent | As 01; FR-090 |
| 03 Has-action filter | 2026-09-28 | As 01 (C9, C22, and a–f). The reference heads its list "Has action · 7" and draws eight rows; the build's heading counts the rows it lists ("Has action · 8" over eight) | As 01; the heading and the toggle read one count (`FocusCounts.has_action`) |
| 04 Open email | 2026-09-28 | Known: the body is drawn by the new renderer from the part the reader draws (C1), and Task and Note wait for Obsidian (C9). The header card and the marker card stay while the body scrolls (research R2), as built. Found: (a) the header card is the shared reader's From and To rows, not a boxed card, and the label pills carry no colour dot; (b) a plain-text body sits on the renderer's paper sheet, a ground and a page, where the reference sets the words on the dialog's white; (c) the toolbar's buttons are the shared action bar's, bordered, not flat labels with bold words; (d) the fold line says "3 quoted lines folded", not "… from v2 folded": the fold knows its line count, not which message it quotes; (e) keycaps are spelled as the keymap spells them, `Escape` rather than "Esc" (C22); (f) the thread chip's `[` is a cap, and the steps are icon buttons with their caps beside them. The marked sentence is highlighted where the body draws it, with a line under it, as drawn | (a) and (c) the shared widgets as they are; (b) the renderer's presentation of plain text; (d) the fold line reads what the document knows; (e) C22 |
| 05 Compose | not yet | No Markdown toggle (C7). Suggestions open at four characters (C23). Cc and Bcc share one command. "Task after sending" waits for Obsidian (C9) | The existing composer; one completion rule |
| 06 Reply all | not yet | As 05 (C7, C9) | As 05 |
| 07 Search, plain English | 2026-09-28 | As 01 (b–f). Typed as "the invoice Marisol sent last month", the demo's correspondent. Known: "invoice" stays a free word, so its chip reads `invoice`, not `subject: invoice`. Found: (a) chips are plain mono boxes with no × and no editing, and Tab does not move into them; there is no "editing after:", no Ctrl ⌫ "back to plain words" and no Ctrl S (the reference's corner note); (b) the saved searches carry no counts; (c) the "Search mail for …" row stays above the results, and the count sits in the heading ("Conversations · 3 matches") rather than at the right; (d) the bar lies 8 px under the strip, over the list, not 60 px from the window's top over the strip too; (e) the footer is one line of text, not caps. Fixed on the way: "Ada"/"Marisol" was a free word because the bar handed `natural::lower` an empty address book (now the account's correspondents); the list did not dim | Research R5; (a), (b) not built yet (T086's chip editing and save; saved-search counts are a count per query); (c) one results list for every kind of row; (d) the bar is an overlay on the list's area; (e) the shared hint spelling (C22) |
| 08 Go to a folder | 2026-09-28 | As 07 (a, b, d, e). `in:Rec` stays typed in the entry rather than becoming an `in: Receipts` chip. The rows have no address column, and the first column is the sender's name as the demo files it ("Receipts") | The bar completes `in:` to a folder and lists it, but does not rewrite what was typed (T086) |
| 09 Commands | 2026-09-28 | Known: "Archive everything read, older than a week" has a key (C11). Found: (a) the command rows are the palette's titles with no detail ("the focused message · Re: …", "3 messages") and no bold on the matched letters; (b) the blend also matches "arch" inside "Search" and "Saved search 1", so those rows are listed where the reference has "Undo: unarchive last"; (c) `in:Archive` has no conversation count; (d) the search row sits under "Go to" with no "Search" heading and no ⇧↵ cap. Fixed on the way: "arch" showed twice, in the entry and as a chip | Constitution II; (a), (b) and (c) `postio_ui::finder::blend` and the palette as they are, shared with the classic app; (d) one list with one heading per kind that has more than one row |
| 10 Folders and labels | 2026-09-28 | As 01 (b, f). The demo store has no Snoozed or Filtered mailbox and fewer folders; it has Junk and Trash, which the reference leaves out. Labels have no counts. The Inbox mark is the icon theme's envelope (Adwaita has no tray), and Archive's a folder. No row is ringed on open; the first place is selected. Fixed on the way: the picture had no popover (a popover is a surface of its own, and the capture drew the window alone); GTK centred the popover on "Inbox", out past the window's left edge; the section heading was lit with the first row; the rows had no marks | The demo store; a label count is a query per label, not read yet; the icon theme |
| 11 Snooze picker | not yet | Preset wording is shared with the classic app (C14). "Comes back at the top" holds only if `sort_at` lands (T012, T093) | ADR 0029; research R7 |
| 12 Remind picker | not yet | none known | |
| 13 Label picker | not yet | none known | |
| 14 Move picker | not yet | none known | |
| 15 Undo toast | 2026-09-28 | Shifted keys (C22). The toast says "Archived 4 messages", not 3: the three selected conversations hold four messages. Its Undo button carries no keycap, and the toast has a close button | C22; the engine counts the messages an action moved; AdwToast's button takes a label only, and its close button is libadwaita's |
| 16 Empty inbox | 2026-09-28 | Lists only what exists (C10), and the strip's own "186 filtered today · 4 digest rules" waits with it. The tray is the icon theme's folder: Adwaita has no inbox icon, so the themed fallback shows. "Inbox is empty" is the reading size, smaller than the reference's. The shortcuts are the shared ghost buttons, their labels bold where the reference's are regular. The sync label is the demo's "Synced 16:09" | C10; the icon theme; the shared button kinds |
| 17 First sync | 2026-09-28 | As 01 (b–f). The progress bar runs the banner's full width under it, where the reference draws a short bar after the sentence. The label reads "Syncing 12,408 of 18,204", as drawn | An AdwBanner holds a title and a button, nothing between; the bar is re-dressed neutral, not the accent's fill (FR-091) |
| 18 Offline | 2026-09-28 | As 01 (b–f). "Retry now" sits at the banner's right edge, where the reference sets it beside the sentence | AdwBanner places its button at the end |
| 19 Sign-in error | 2026-09-28 | As 18. The server and the address are the demo account's. "Update password…" does not open anything yet | As 18; the credential dialog is still `postio-gtk`'s (T055) |
| 20 Key map | 2026-09-28 | Known: the footer names `[keys]` in `config.toml` (C3), no Obsidian group (C9), keys spelled as the keymap spells them, `ctrl+k` rather than "Ctrl K" (C22). Found: one row per command, each with every binding, alternates included, where the reference merges pairs ("Top / bottom", "Extend selection") and shows one or two keys; and the groups hold every command Focus offers in the key map's contexts, zoom, find in message, the outbox's verbs and the picker's number keys among them, where the reference shows a shorter set. So the columns run past the dialog and scroll | Constitution II: the key map is generated from the registry and the groups table (`postio_ui::keymap_sheet`), and a command Focus offers is taught; merging pairs, or leaving a row out, is a change to that table rather than to the dialog |

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
