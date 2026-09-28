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
| 04 Open email | not yet | The body is rendered HTML, not the plain-text part (C1). The header and marker cards stay while the body scrolls (research R2). Task and Note buttons wait for Obsidian (C9) | Handoff; one reused view; milestone order |
| 05 Compose | not yet | No Markdown toggle (C7). Suggestions open at four characters (C23). Cc and Bcc share one command. "Task after sending" waits for Obsidian (C9) | The existing composer; one completion rule |
| 06 Reply all | not yet | As 05 (C7, C9) | As 05 |
| 07 Search, plain English | not yet | "invoice" stays free text rather than `subject:` | Deterministic lowering (research R5) |
| 08 Go to a folder | not yet | none known | |
| 09 Commands | not yet | "Archive everything read, older than a week" has a key (C11) | Constitution II |
| 10 Folders and labels | not yet | none known | |
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
