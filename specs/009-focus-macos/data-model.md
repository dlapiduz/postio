# Data model: Postio Focus on macOS

This feature adds no store tables and no columns. The entities below are
in-memory state, or records that cross the FFI. Engine-side types are named
as they are on `main`.

## FocusController (crate `postio-focus`)

One per window, owned by the frontend's driver. It is `Send`; its fields are
plain data with no `Rc` and no GObject.

| Field | Type | Meaning | From (today) |
|---|---|---|---|
| `policy` | `Policy { platform, caps }` | Linux or Apple, plus capabilities: `reading_pane`, `stacking` (Linux stacks dialogs, the Mac has one window, M4) | new |
| `resolver` | `postio_ui::keymap::Resolver` | Focus's keymap for this platform; rebuilt on `Keymap` input | `gtk/keys.rs:14`, `ffi session.rs:417` |
| `cursor` | `Cursor { position, id, keep, placing }` | where the keyboard is in the list; `keep` survives a `!` toggle; `placing` suppresses the toolkit's echo | `window.rs:152-168` |
| `selector` | `postio_ui::selection::Selector` | what `a` acts on; anchor is a `MessageId` | `selection.rs:61-212` |
| `place` | `Place { scope: ListScope, has_action, counts: Option<FocusCounts> }` | the list shown and the strip | `window.rs:160-215` |
| `feed` | `Feed { generation, paging, spliced, surfaced, total, landed, opened, pages_asked }` | list loading state | `list/feed.rs:37-60` |
| `removal` | `Removal { removed: ≤64 ids, restoring: Option<Restoring>, step_past }` | cursor placement after removal and undo | `window.rs:98-105, 2144-2215` |
| `sync` | `SyncState { trackers, facts, last_synced }` | banners and the sync label | `window.rs:169-178` |
| `stack` | `Vec<Surface>` | open surfaces, top last; the top decides the key context | widget names, `window.rs:877-1016` |
| `tickets` | `Tickets { next, outstanding }` | open requests, with generations | new |

### Surface (one element of `stack`)

| Variant | State |
|---|---|
| `Message` | `shown: MessageId`, `origin: List \| Found{hits} \| Digest{delivery} \| Filtered`, `position`, `treatment: Treatment`, `more_open`, `finding` |
| `Digest` | `delivery`, `page: Summary \| List`, `focused_reference: Option<u32>`, `email: Option<MessageId>` |
| `Filtered` | `tab: Option<String>`, `counts`, `offset`, `more` |
| `Capture` | `mode: Task \| Note`, `source: MessageId`, `due`, `project` |
| `Bar` | `mode: Search \| Commands`, `words`, `editing`, `shown_chips`, `aim: Aims` |
| `Places` | `filtered_today` |
| `Picker` | `kind: Snooze \| Remind \| Label \| Move`, `anchor: Row(pos) \| OpenMessage`, `aim: Aims` |
| `RowMenu` | `position`, `alone` |
| `KeyMap`, `Rules`, `RuleDialog`, `Composer{draft}`, `Settings`, `Credential`, `Confirm(kind)` | only what each needs to close, plus its aim |

**Transitions.**
- *Opening* pushes a surface.
- *On the Mac* (`stacking = false`), opening a Message, Digest, Composer or
  Capture first pops any of those four. A Message opened from a Digest is
  not pushed; it sets `Digest.email` (M4).
- *Back* pops the top surface. On the list, Back follows the ladder: places,
  then bar, then filtered, then clear the selection (R2).
- *Closing* any surface emits `KeyboardHome(CursorRow)`.

## Input, Effect, Intent, Request, Reply

These are defined in [contracts/focus-controller.md](contracts/focus-controller.md).
Validation rules:

- A `Reply` whose ticket's generation is not current is dropped, so stale
  pages never land.
- `press` while an input method is composing, or with `in_text_entry` and a
  plain printable key, returns `handled: false` with no effects (FR-032).
- An `Intent` refers to rows by position and id together. The host trusts
  the id and re-finds the position if its window moved.

## Window geometry (`postio_ui::focus_dialog`, keyed by `Platform`)

| Quantity | Linux (unchanged) | Apple (M1) |
|---|---|---|
| message width from main width `W` | current formula, max 820 | `clamp(640, W − 2·max(96, 0.18·W), 720)` |
| message height | `H − 80` | `H − 80` |
| text column | `min(480, w − 96)` | `min(560, w − 80)` |
| paper column | `min(640, w − 48)` | `min(640, w − 80)` |
| More fold | below 760, or when the row does not fit | below 700 |
| digest window | 980 × 820 | the message window's size, column 560 |
| paper zoom floor | 0.85 | 0.85 |

Test values (Apple): W = 1024 → 656; 1280 → 720; 1440 → 720; 1920 → 720. At
1440 the text column is 560 and paper is 640. At 1024 the text column is 560
and paper is 576 (a 640 newsletter at 0.9).

## Records that cross the FFI

All are new `*Ffi` mirrors in `postio-ffi`; engine types derive nothing from
uniffi. The full list is in [contracts/ffi-focus.md](contracts/ffi-focus.md).
Conventions:

- Ids are `i64`.
- Times are `i64` Unix seconds (UTC), with local formatting done in Rust
  where words are shown.
- A `PathBuf` becomes a `String`.
- A tuple becomes a named record.
- Enums with data become uniffi enums.

## Undo entry

The engine's `UndoEntry` is unchanged. Two new reads expose its top:
`Actions::peek_description() -> Option<String>`, and `Req::UndoTop`. The Mac
caches the description and refreshes it on `Notice{Completed|Undone}`.
`canUndo` is `description != nil`.

## Body treatment

`postio_body::treatment::Treatment { AppColours, Paper }` is unchanged. The
reader document record gains:

- `treatment_shown`
- `treatment_classified`
- `render_mode_words` (the quiet line above the body)
- `sender_choice: Option<Treatment>`

Switching (⇧O) and "Always for this sender" go through the existing
allowlist file (`postio_ui::allowlist`, `[Treatment]`).
