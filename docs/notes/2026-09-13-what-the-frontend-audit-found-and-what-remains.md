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
3. **Notification wording** — *done, same day.* `postio_ui::notify::decide`
   is the one rule (suppression, identifier, click target); `postio-app`
   and the FFI's `decideNotification` are shims over it. What stayed a
   per-platform choice, on purpose, is the **wording**: macOS draws counts
   and a folder name because a notification there is a log the lock screen
   reads out; GTK draws the newest sender and subject because the shell keeps
   banners off the lock screen and a popup saying nothing about the mail is
   not worth the interruption (#745). Both are `Wording` variants, so the
   choice is visible rather than two divergent copies. Still open: the
   `[sync] notify_roles` gate does not cross the boundary, so macOS notifies
   for every folder as it always has.

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
- Bulk test fixtures pay the fts index per document, so
  `total_hits_stops_counting_at_the_cap` (10,050 repository inserts) runs
  2-5 minutes and carries a nextest timeout override. A bulk loader that
  seeds `messages`/`search_documents` in multi-row statements would take
  most of that back.
- `postio-body/src/styles.rs` (501 lines over `cssparser`) may duplicate
  `ammonia`'s style filtering — unconfirmed; check before touching.
