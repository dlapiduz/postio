# What the frontend audit found, and what remains

2026-09-13, from a full audit of the GTK↔engine boundary and the widget
layer, run while the Turso branch was hardened. The findings that were acted
on are in the branch's commits (the chip constructors, the kicker collapse,
`postio_ui::{list_state, status, cheatsheet}`, the FFI's
`cheatSheetSections`). This note is the part that was mapped and *not* done,
so the next session starts from the map rather than the audit.

## The boundary, as it stands

`postio-gtk` genuinely cannot reach the store — the checks hold. The leak is
subtler: frontend-agnostic *decisions* living in GTK files, which the macOS
frontend (`crates/postio-ffi` + `macos/`) then re-derives or goes without.
Three moves remain, in value order:

1. **The store→list source seam.** `MessageSource` / `MailboxSource` /
   `ResultSource` / `PageRequest` / `Page` are defined in
   `postio-gtk/src/feed.rs`, so `postio-app/src/feed.rs` implements a GTK
   trait to serve data, and `postio-ffi/src/session.rs` (~1697–1800) had to
   hand-roll its own fetch/in-flight/generation logic. Move the traits to
   `postio-ui` generic over `ListRow`, apply `ListScope::reaction` in a
   shared driver, leave `postio-gtk::feed::Feed` as the adapter. Largest
   move, deletes a duplicated subsystem, gives macOS the
   insert-at-top/refetch/ignore distinctions it currently flattens into
   `reloadData()`.
2. **The focus/keyboard-context machine.** `window.rs` ~2455–2530 owns the
   pane-cycle table and context stack; `macos/Sources/Postio/Engine.swift`
   ~420–460 re-derives all of it. A `postio_ui::focus` with
   `cycle`/`enter`/`leave` beside `keymap` ends the double bookkeeping.
3. **Notification wording.** `postio-app/src/notifications.rs` ~250–301 and
   `macos/Sources/PostioKit/MailNotifier.swift` ~59–95 are two products
   today (different titles, different click targets, macOS-only
   suppression). One `decide()` in `postio-ui`, two thin shims.

## The widget layer

Chips and kickers are constructors now. Still hand-rolled, with counts:

- **name+count rows** ×4 (`sidebar.rs` folder_row/tree_row, `search.rs`
  scope_row, `finder.rs` row_shell) — spacings 10/6/10/12, one
  `count_row()` collapses them.
- **empty-state notes** ×7 over three CSS classes with diverging padding —
  these are mostly template children, so the fix is CSS-level (one
  `.postio-empty`) plus adoption, and it needs a per-pane look at the canvas
  before changing insets.
- **key-hint pairs** ×5 bare `postio-keyhint` sites beside the
  `KeycapButton` that exists for the purpose, plus `orientation.rs`'s
  `chip()`.

## Also known and deliberately left

- The engine's fts merge **cost** (tantivy inside a committing transaction)
  is measured but untamed — the wave scheduler now survives it (see
  `2026-09-13-a-slow-pass-stops-every-folder-behind-it.md` and the
  `yield_once` fix), but a merge-laden pass is still a slow pass. turso
  0.8.0-pre.11 exposes no merge tuning.
- `zbus` (~35 crates) exists for one 175-line NetworkManager watcher
  (`postio-runtime/src/network.rs`) — the workspace's worst
  dependency-per-line ratio, kept because online/offline detection has no
  cheaper honest source. A decision, not drift, as of this audit.
- `postio-body/src/styles.rs` (501 lines over `cssparser`) may duplicate
  `ammonia`'s style filtering — unconfirmed; check before touching.
