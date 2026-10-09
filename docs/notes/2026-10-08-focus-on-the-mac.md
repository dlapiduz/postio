# Focus on the Mac: what the next session must keep

2026-10-08, specs/009-focus-macos T127. The constraints the Mac build of
Focus stands on, gathered from the phase notes and the "As built" entries in
`specs/009-focus-macos/tasks.md`. Break one and a test in some layer will
usually still pass; the person at the keyboard is the one who notices.

## Constraints

- **One secondary window at a time (M4).** The message, the digest, the
  composer and capture share `SecondaryWindowController`. Opening another
  kind closes the first and reports it closed; the same kind again replaces
  the content and keeps the frame. Do not open a second one from a view, and
  do not add a surface that bypasses it.
- **The intents, not the widgets.** Behaviour (where the cursor goes, what a
  key does, what opens, what the keyboard returns to) lives in the
  `postio-focus` controller. Swift applies its intents through
  `FocusIntents` and draws them. A rule that wants changing is changed in
  the controller, with a test there first, seen red; a Swift branch that
  decides something is on the wrong side (`macos/CLAUDE.md`).
- **Surfaces the controller opens are not echoed back.** Reporting
  `focusSurfaceClosed` for a close the controller made can land after a
  reopening and close the new surface (the bar, phase 5).
- **The treated document comes from the shared Rust.** The message body is
  `focus_reader_document`, with the contrast guard in the markup (a
  sender's red survives it, R9). Swift does not recolour, and does not size
  a web view by `scrollHeight`, which the classic reader's `vh` scroll
  anchors inflate.
- **Words come from `postio-ui`**, through the FFI: labels, hints, footers,
  key spellings, toasts. A Swift string literal that a person reads is a
  second copy that will drift (C22).
- **GTK has not adopted controller slices 3 to 12.** Adoption needs a Linux
  session, because this Mac cannot build `postio-gtk` and editing
  `window.rs` blind was judged worse than waiting. Until then GTK and the
  Mac run two copies of those rules; the `focus_suite` guards keep GTK
  honest. Each slice's `Open:` entry in tasks.md names what is owed.
- **Swift reports only toolkit-only facts.** Clicks, the list scrolled to
  its top, a surface the toolkit closed (a click outside a popover or on a
  dimmed area), and reader state. Everything else the controller already
  knows.

## Found on the way

- **`rustdoc --message-format=json` keeps spans that the text output
  drops** (found 2026-10-08). When a doc-link or `cargo doc` warning shows no
  location in the text form, ask for JSON and read the span there.
- **WebKit does not paint a web view in a window it thinks is covered.** A
  demo or a capture whose window opens behind another app photographs a
  blank body. `macos-shot.sh` and the demo open in front; keep it that way
  (phase 3).
