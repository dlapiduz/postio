# Focus on the Mac, phase 4: the composer against screens 05 and 06

2026-10-08, specs/009-focus-macos T080 (FR-061). The fourth Mac comparison,
for US4 (phase 6 of tasks.md), the one the comparisons' numbering left out.

**Captures.** `scripts/macos-shot.sh` at 1440×900, light and dark, the state
reached with `POSTIO_DEMO_KEYS` through the resolver and `invoke`, as a
press would go:

| Capture | Seed | Keys | Reference |
|---|---|---|---|
| `05-compose-new` | `small:05` | `c g r a c` | screen 05 |
| `06-compose-reply-all` | `small:06` | `j j j E` | screen 06 |

Captures live in the main checkout's untracked `Design/review/focus-macos/`.

**Demo additions** (demo builds only). `small:05` and `small:06` add
`postio_demo::compose_demo`'s people and the Harbor thread's recipients and
body, as GTK's `shot` does for its 05 and 06. A demo's composer types
characters into To (`Engine.replayIntoField`), so the completion list can be
photographed, and never reads the address book: a photograph is no time for
the system's Contacts prompt, and a demo's mail is invented.

## What matches

- **The window.** Its own titled window over the undimmed list, one at a
  time (M4), 980 wide at 1440 as screen 05 is: the main window less 80 below
  that. Opened by the controller (`FocusComposer`), from `c`, `E` and the
  toolbar's compose button alike.
- **The title area.** "New message" or "Reply to all" in the centre, "Draft
  saved locally 08:00" under it once the controller's autosave has landed
  (the demo's typing armed it, 1500 ms after the last key); Send later ▾ and
  Send with its ⌘↩ cap on the right, Send a default button filled in the
  label colour, turned over in dark.
- **The rows.** From with the account's name on the left and its address,
  a picker, on the right; To; Subject. The rows' labels in the secondary
  colour, hairlines between them, 42pt rows.
- **Completion (05).** Typing in To shows the list under the field, the
  first suggestion ringed in the accent; the rest one row each. Nothing shows
  before a recipient is typed, and a reply opens with none (06).
- **The reply (06).** Pre-filled: To the sender, Cc the others, the subject
  "Re:", the caret in the body, and the quote folded under it: "› On
  2026-10-08, Tobias Wren wrote · 1 quoted line  show".
- **The footer.** Attach ⌘⇧A, Remind if no reply ⌘H, and the word count on
  the right ("Plain text · 24 words").

## Differences, and why

- **Labels row** (05, 06): not drawn. The draft carries no labels across the
  boundary, and a reply's labels from its thread are GTK's composer seam
  (`reply_source`), not the shared model's. Deferred, below.
- **Recipient chips** (05, 06): the fields are text, as the Mac's composer
  has always been; screen 05 draws each address as a removable chip. The
  text is what the boundary parses and what `accepted` replaces, so chips
  are drawing, not behaviour; deferred.
- **Completion from four letters** (05): the design shows the list after
  "gra"; the shared rule (`postio_ui::recipients::MIN_COMPLETION_PREFIX`,
  GTK's and the terminal's too) completes from four, so the capture types
  "grac". Kept: one rule for every app.
- **The list's rows** (05): one line, "Name <address>", without the design's
  "wrote 42 times" or "list" on the right, or its footer of keys and sources.
  `RecipientSuggestionFfi` carries the label and whether it is a group, not
  the count; the group's "group" word is drawn.
- **"Cc · Bcc ⇧⌘C"** (05): the design draws "Cc ⌘⇧C · Bcc ⌘⇧B". The registry
  has one command for both fields (`copy_fields`), so the key is said once,
  in C22's spelling.
- **Cc and Bcc together** (06): a reply-all with copies shows both rows,
  since the one key shows both; screen 06 draws only Cc.
- **Rich/Plain** (05, 06): the switch sits in the footer beside the word
  count, and the marks bar shows only for a rich draft. Rich composition is a
  shipped feature (#1271) the design does not draw; it is kept, out of the
  way.
- **The fold's place** (06): it sits at the foot of the body's area rather
  than right under the last line written, since the plain body is a
  `TextEditor` that fills the window. Its words are the design's except the
  ⌘⇧Q cap, which is M3 (Phase 12); "show" is a click for now.
- **Task after sending ⌘T and Markdown ⌘M** (05, 06): not drawn; M3
  (Phase 12).
- **Fixtures.** The demo's account is "Test" at `test@example.com`, its
  reply-all copies the demo's own `you@example.com`, and its people and
  subjects are the demo's, not the pack's.

## Found and fixed on the way

- A reply whose quote ended in a newline did not fold: `fold_quote` looked
  for quoted lines at the very end. Blank lines after the quote are the
  quote's now (`da92d499`).
- The reply's keyboard landed in To with its text selected: SwiftUI's
  focus, set as the view appeared, lost to AppKit giving the window's first
  field the keyboard. It is set a moment later, once the window is key.
- The reminder's ⌘H cap vanished inside the menu's label (a menu draws its
  label as text); it stands beside the menu.

## Deferred

- The Labels row: `DraftFfi` needs the draft's labels (ids and names), the
  reply needs its thread's, and the composer a label picker.
- Recipient chips, and the list's "wrote N times" (an FFI field) and footer.
- A queued draft opened with Edit is not taken off the queue first on the
  Mac, as GTK's `resume` does ("Send cancelled -- you're editing this draft
  again").
