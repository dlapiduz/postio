# Contract: The Observation, per App

`postio_ui::observe::Observation` is defined in
[../data-model.md](../data-model.md) § Observation. This contract says how
each app fills it. Every app MUST fill every shared field from the source
named here, or declare it `None` ("not observed by this app"). It MUST NOT
guess.

These are **rules every frontend must obey**, and they outlive this feature.
That is why ADR 0044 records them (plan § ADR).

## Rules common to every app

1. **`observe()` reads; it never acts.** No store read, no command, no focus
   change, no layout pass beyond what the toolkit already has queued. It is
   called after every step, and an observation that changes what it observes
   is worthless.
2. **`keyboard.region` comes from the real focus widget**, walked up to the
   first ancestor the app names as a region. If the focus widget is gone, the
   region is `none`, and `keyboard.reachable` is `false`.
3. **`keyboard.typing` is the resolver's own flag.** It is the same value the
   app passes to `Resolver::press`, so the observation cannot disagree with
   what the keymap decided.
4. **Overlays.** `overlay.kind` names the *topmost* thing that takes the
   keyboard over the main window: a dialog over a palette is `dialog`.
5. **Notices.** `notice.tone` and `notice.undo` are recorded when the notice
   is shown. They are not inferred from its label (research R6).
6. **Ids are store ids**, as strings. A seed makes them stable.

## Classic (`postio-gtk::window::Window::observe`)

| Field | Source |
|---|---|
| `view` | `context()` and `reader_occupant()` (`shell.rs:378`): `Conversation` → `conversation`; `Composer` → `composer`; `SearchPreview` → `search`. Settings window open → `settings`. Orientation showing → `first_run`. Locked store → `locked`. Otherwise `list`. |
| `scope` | `scope()` (`window.rs:3076`), by display name |
| `keyboard.region` | The focus widget's ancestors. `sidebar`, list view → `list`, reader view → `reader`, conversation → `conversation`, composer → `composer`, finder → `search` or `picker` (mailbox mode), palette, cheat sheet → `cheatsheet`, settings window → `settings`, `adw::AlertDialog` → `dialog`. |
| `keyboard.field` | `composer().focused_field()` (`composer.rs:897`), when the region is `composer`. `query` when the region is `search`. |
| `keyboard.typing` | `is_typing()` (`window.rs:2960`, private, so read inside `observe`) |
| `keyboard.reachable` | The shared GTK half's `reachable(&window)` (research R3) |
| `cursor.*` | `list().cursor().selected()`, `cursor_id()` (`list_view.rs:357, 914`), and the row's subject |
| `rows.*` | The list model's `n_items()`, and the first visible row from the list's vadjustment |
| `selection.count` | `list().selection()` (`list_view.rs:362`) |
| `overlay.*` | `finder().is_open()` + `mode()` (`finder.rs:578, 588`); `cheatsheet().is_visible()`; palette; any presented `adw::Dialog` |
| `notice.*` | `toast().showing()` (`toast.rs:74`), plus the new `tone()` and `offers_undo()` |
| `banner.title` | The connection or sync banner, if one is showing |
| `reading.id` | `reading()` (`window.rs:1365`) |
| `reading.focused` | `conversation().focused_index()` (`conversation.rs:2552`) |
| `reading.scroll` | `reader().view()`'s vadjustment, or the conversation's one-document view |
| `composer.*` | `has_composer()`, `composer().is_open()`, detached state |
| `back_depth` | `None` (Classic's Back is a cascade, `window.rs:2732`) |
| `app.classic.pane` | `shell().focused_pane()` (`shell.rs:554`) |
| `app.classic.reader_occupant` | `reader_occupant()` |

## Focus (`postio-focus::window::FocusWindow::observe`, on `feature/postio-focus`)

| Field | Source |
|---|---|
| `view` | `reading()` open → `reader`; `digest()` showing → `digest`; `filtered()` active → `filtered`; `bar()` open → `search`; composer dialog → `composer`; otherwise `list` |
| `keyboard.region` | Focus widget ancestors. List pane → `list`; open dialog → `reader`; bar → `search`; places → `picker`; `open_picker()` → `picker`; `row_menu()` → `menu`; key map dialog → `dialog`; rule dialog → `dialog` |
| `keyboard.typing` | The same `typing` that `handle_key` computes (`window.rs:611`) |
| `cursor.*` | `pane().cursor().selected()`, `cursor_row()` (`window.rs:1515`) |
| `selection.count` | `selection()` (`window.rs:1510`) |
| `overlay.*` | `bar().is_open()`, `places().is_open()`, `open_picker()`, `row_menu().is_open()`, `key_map()`, `rule_dialog()` |
| `notice.*` | `toast_showing()` (`window.rs:3741`), plus the new `tone()` and `offers_undo()` |
| `banner.title` | `banner_showing()` (`window.rs:2925`) |
| `reading.*` | `reading()` (`window.rs:3351`); scroll from `reader().view().scrolled_for_test()` (focus: `postio-widgets/src/reader/view.rs:1585`) |
| `back_depth` | `None` (Back is a cascade, focus: `window.rs:1263`) |
| `app.focus.bulk` | Whether the bulk bar is shown, and its summary. This needs a getter; `bulk.rs:102` has only a setter. |
| `app.focus.digest_page` | `digest().showing()` |

## Later apps

These are not built here. FR-031 requires that they can be.

- **Terminal**: `postio_tui::App` getters. `focus()` → region, `cursor()`,
  `selection()`, `palette()`, `cheat_sheet()`, `notice()`, `notice_tone()`,
  `notice_undo()`, `reading()`. `back_depth` comes from `state()`'s back
  stack, which the TUI does keep.
- **macOS**: an FFI-exported `observe()` filled on the Swift side, through the
  same struct generated by UniFFI.
