# Focus on macOS: a starting brief

For whoever builds the Focus design in `macos/`. It is a starting point and
not a spec: the spec is [`spec.md`](spec.md), and where this file and the
spec disagree, the spec is right.

## What Focus is

Postio's one desktop app ([ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md),
decision C27). Home is a dense, keyboard-first inbox that shows mail as it
arrived. It does four things to mail: it calls out real actions (invitations,
questions, to-dos), holds some mail back into digests on a cadence the person
chooses, hides spam and automated updates with a reason each, and later links
mail to an Obsidian vault. It has no folder sidebar. A message opens in a
dialog over the list, or beside it for those who choose (`F8`), and `Esc`
returns to the same place.

Read these before drawing anything:

- [`spec.md`](spec.md): the user stories, the requirements, and the table
  *Where the inputs disagree* (C1 to C27), which records every decision the
  maintainer has taken against the design files;
- [`screens.md`](screens.md): each screen against its reference image, with
  every known difference and its reason, and the designs that have no image
  (the open message, the reading pane, sending states, the row menu,
  Settings);
- [`contracts/focus-surface.md`](contracts/focus-surface.md): rows, markers,
  the bar, the pickers, dialogs and states, as a frontend must draw them;
- [`contracts/keymap.md`](contracts/keymap.md): the one keymap;
- the open message: [`screens.md`](screens.md), "The open message", with
  decisions C25 and C26: the dialog's size from the window, the one centred
  column, the vertical rhythm, and the two body treatments, app colours or
  the original on a paper sheet.

## What macOS can use as it is

The engine is shared, and none of Focus's engine work is GTK:

- **The store, sync and the host.** Focus's tables (markers, filter
  decisions, digests, reminders, classification) are in the one store, and
  the first sync, inbox-first ordering and filing run in the engine. The FFI
  session already runs a `postio_host::Host` on the same store, so a Mac
  build gets the filing and the markers without doing anything.
- **`postio-client`'s requests.** Focus reaches mail only through `Req`/
  `Resp` (ADR 0041), for example `FocusCounts`, `Surfaced`, `FilteredTabs`,
  `Filtered`, `Held`, `DigestSummary`, `DigestPreview`, `SaveDigestRule`,
  `SweepPreview`, `RawSource`, `Vault`, `CaptureTask` and `CaptureNote`, plus
  `ListScope`'s Focus lists in `postio-model::scope`. The FFI does not export
  these yet, but the host behind it answers them.
- **`postio-ui`'s toolkit-free presenters.** `focus_row` (a row's words and
  its marker line), `focus_state` (what the inbox says when empty, syncing,
  offline or failing), `focus_dialog` (the dialog's width and height from the
  window, the column width for each treatment, the rhythm), `surfaced`,
  `filtered`, `digest`, `pickers`, `keymap_sheet` (the key map dialog's
  groups), `reader::document::render_mode_words`, and `allowlist` (remote
  images, and each sender's chosen treatment, in one file).
- **`postio-body::treatment`.** `classify` decides whether a sanitised body is
  drawn in app colours or on paper, and `app_colours`/`app_colours_css` strip
  a sender's colours and fonts. The Mac's `WKWebView` can apply the same
  decision and the same stylesheet (`postio-ui/data/treatment.css`).
- **The registry.** `Frontend::Focus`, its `Requirement`s and the Focus-only
  commands (`switch_treatment`, `more_actions`, the pickers, the digest and
  filtered commands) are rows of the one table. `offered_on` already scopes
  what a platform has no surface for.
- **Store refusals.** `Session.open` raises `SessionError.
  storeFromAnotherBuild` when the store's schema cannot be carried forward,
  which is distinct from `storeUnavailable` (try again) and `keyringLocked`.

## What macOS lacks for the Focus design

- **A Focus surface over the FFI.** The FFI session is the classic
  three-pane frontend's: sidebar, list over a mailbox, conversation, reader
  and composer. Nothing exports the Focus inbox, its markers, the filtered
  view, digests, the rule dialog, the pickers at the row, or `FocusCounts`.
  These want FFI types over the `postio-client` requests above, not new
  engine work.
- **The Focus frontend in the registry.** The FFI asks with
  `Frontend::Macos`, which `Requirement::ThreePane` treats as a classic
  frontend. A Focus build on the Mac has to ask as `Frontend::Focus`, or be
  given a `Requirement` that means "Focus's design", and the Mac's
  `offered_on` scoping must be checked against Focus's command set.
- **Starting the store over.** The error is mapped, but no FFI call starts
  the store over (`postio_session::start_over`), so the Mac can say why it
  cannot open and cannot yet offer the way forward.
- **The message dialog and its treatments.** The Mac reads mail in a web view
  with no recolouring pass (`offered_on` withholds `darken_message` from it).
  App colours needs `treatment::classify`, the stylesheet and the contrast
  guard (`postio_render::theme::guard`), or an equivalent the maintainer
  accepts. Paper needs the white sheet and the fit-to-column zoom down to
  0.85.
- **Six commands every app is offered.** `go_to_archive`, `go_to_snoozed`
  and `saved_search_1` to `saved_search_4` reach nothing on the Mac; they
  are `KNOWN_ORPHANS` in `ffi_suite/command_coverage.rs` until a surface
  answers them.
- **Invitations, reminders, the vault and the local model.** The engine does
  these (`postio-calendar`, `postio-ai`, `postio-vault`), and the FFI exports
  none of them.

## Rules for every frontend

- **C25, the system font.** The chrome is in the system's sans and mono
  faces (Adwaita Sans and Adwaita Mono on GNOME; on the Mac, the system
  equivalents), at the design's sizes, weights and gaps. Barlow is used only
  for a message body drawn in app colours.
- **C26, the system accent.** Focus has no accent of its own: the cursor
  ring, the markers, the action card's fill (the accent at 8% light, 12%
  dark), links and tags take the platform's accent colour.
- **One keymap, filtered per frontend** (C20, FR-081, `contracts/keymap.md`).
  Every app reads the same registry and gives each command the same key.
  What differs is which commands a frontend offers (`Requirement` against
  `Frontend`) and what a platform has no surface for (`offered_on`). `mod`
  resolves to `cmd` on the Mac, and the Mac's tests assert the one keymap's
  defaults: `]`/`[` for the next and previous message in a thread, `mod+z`
  for Undo.
- **ADR 0043.** Focus is the one desktop app. New work is Focus's design or a
  shared crate's, never the classic three-pane app's. A capability goes in
  the lowest layer that can hold it: `postio-ui` or `postio-core` for logic
  with no toolkit, which is also how the Mac gets it for free.
