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

## Focus (`postio-gtk::window::FocusWindow::observe`)

Postio's window is the one every storyboard is played on (ADR 0043;
specs/007-postio-focus T265).

| Field | Source |
|---|---|
| `view` | An unavailable page (`unavailable_reason()`: a locked keyring, a store in use) → `locked`; Settings open → `settings`; the composer showing, in its dialog or the reading pane → `composer`; the add-account dialog → `first_run`; `reading()` open, in its dialog or the reading pane → `reader`; `digest()` showing → `digest`; `filtered()` active → `filtered`; `bar()` open → `search`; otherwise `list` |
| `scope` | `place_name()`: the header strip's "Inbox", "Drafts", "Flagged", "Outbox" |
| `keyboard.region` | The focus widget and the dialog over the window. `row_menu()` → `menu`; places or `open_picker()` → `picker`; Settings → `settings`; key map, rule dialog, raw source, open chooser, capture sheet, add account → `dialog`; the composer → `composer`; the open message, in its dialog or beside the list (T232) → `reader`; the digest window → `dialog`; the bar → `search`; the list pane → `list`. With focus on nothing, or on a control in the top bar or foot strip while the list's context holds, `list`: the window's capture-phase controller takes every key first, and in that context they are the list's |
| `keyboard.field` | The composer's `focused_field()` (`to`, `cc`, `bcc`, `subject`, `body`) when the region is `composer`; `query` when it is `search` or `picker` |
| `keyboard.typing` | `is_typing()`, the same answer `handle_key` gives the resolver |
| `keyboard.reachable` | The shared GTK half's `reachable(&window)`, or nothing focused and nothing over the window |
| `cursor.*` | `pane().cursor().selected()`, `cursor_row()`; a digest row's subject is its rule's name |
| `rows.count` | The list's `n_items()`. `rows.first_visible` is not observed: the list is windowed over the store |
| `selection.count` | `selection()`, an everything-but predicate counted against the rows |
| `overlay.*` | Key map → `keymap`; rule dialog → `dialog`/`rule`; Settings → `dialog`/`settings`; raw source and the open chooser → `dialog`/`raw-source`, `open-choice`; `row_menu()` → `menu`; `open_picker()` → `picker`; places → `picker`/`places`; the bar → `finder` |
| `notice.*` | `toast_showing()`, with the tone and undo the toast recorded when it was shown |
| `banner.title` | `banner_showing()` |
| `reading.id` | The open message's `shown()`, while the view is `reader` |
| `reading.scroll` | The open message's `scroll_extent()`. `reading.focused` is not observed: one message is shown at a time (C2) |
| `composer.*` | `open` as `view`; `detached` is `false` |
| `back_depth` | `None` (Back is a cascade) |
| `app.focus.bulk` | Whether the bulk bar is shown, and its summary (`Bulk::summary`) |
| `app.focus.digest_page` | `digest().showing()` |
| `app.focus.bar.typed`, `.heading`, `.messages` | The bar's field, its results heading ("Conversations · 8 matches") and how many messages it lists: the hits, which the list behind it never shows |
| `app.focus.bar.highlighted`, `.highlighted_id`, `.highlighted_text` | The row Return would run: its kind (`message`, `order`, `search`, `command`, `place`, ...), the message it opens, and what it says |

An app field is named by its whole key (`app.focus.bar.messages`), one key
per value, because a check reads the rest of its path after `app.` as one
key.

## Later apps

These are not built here. FR-031 requires that they can be.

- **Terminal**: `postio_tui::App` getters. `focus()` → region, `cursor()`,
  `selection()`, `palette()`, `cheat_sheet()`, `notice()`, `notice_tone()`,
  `notice_undo()`, `reading()`. `back_depth` comes from `state()`'s back
  stack, which the TUI does keep.
- **macOS**: an FFI-exported `observe()` filled on the Swift side, through the
  same struct generated by UniFFI.
