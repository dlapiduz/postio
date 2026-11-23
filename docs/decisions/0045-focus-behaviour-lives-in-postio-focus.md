# ADR 0045 — Focus's behaviour lives in `postio-focus`; frontends draw its intents

- **Status:** Accepted (2026-10-07), with `specs/009-focus-macos`; amended 2026-11-22
- **Decision:** What Focus does is decided once, in `postio-focus`, by a controller that does no I/O. A frontend turns key presses, engine events, client replies and facts about its own window into controller inputs and draws the intents that come out. No rule about what a command does, which surface a key reaches, where the cursor goes or what a surface holds lives in a frontend's window code.

## Rules
- `postio_focus::FocusController` owns the cursor, selection and current place; the stack of open surfaces; each surface's state; and the Focus key resolver. It is `Send` and holds no toolkit object.
- It is sans-IO and never awaits. It emits `Effect::Show(Intent)`, `Effect::Ask(ticket, Request)` and `Effect::Timer`. `postio_focus::perform(&Client, Request)` maps a request to the client. The frontend runs it on its own executor and feeds the reply back with its ticket. A reply for a stale generation changes nothing.
- Frontends are drivers: `postio-gtk`, `postio-ffi` and the terminal frontend `postio-tui`, and any later one. They resolve nothing themselves, apply intents in order, and report facts the controller cannot see (window size, a pointer placing the cursor, a surface closed by its own button). A behaviour a frontend needs and the controller lacks is added to the controller, with its test, first.
- Platform differences are policy, not forks: `Policy { platform, caps }` covers geometry (spec 009 M1), whether secondary surfaces stack or replace (M4) and whether reading beside the list exists. The rules are the same.
- `postio-focus` may depend on `postio-ui`, `postio-client`, `postio-core`, `postio-model`, `postio-config` and `postio-search`. It may not depend on:
  - a toolkit: GTK, GDK, libadwaita, webkit, their `-sys` crates, or AppKit by any route;
  - the store or protocol: `turso*`, `rusqlite`, `libsqlite3-sys`, `io-imap`;
  - the crates that own them: `postio-host`, `postio-session`, `postio-storage`, `postio-runtime`;
  - the frontends: `postio-widgets`, `postio-gtk`, `uniffi`.
- It may not name `tokio`, `glib` or `async-std` as a direct dependency. `check-crate-boundaries.py` enforces the list.
- `postio-ui` keeps pure words and decisions (`focus_row`, `focus_dialog`, `focus_state`, `selection::Selector`, the key resolver), with no stateful owner and no client.

## Why
- A module in `postio-ui` cannot name a client type without a cycle. A controller that owns a `Client` is tied to one executor (glib for GTK, tokio for FFI) and makes every rule an integration test.
- Per-frontend rules drift: the FFI's `Back` cleared only a selection, and its anchor was a row index where GTK's is a message. GTK window types cannot cross the FFI.

## Revisit when
- A frontend needs a behaviour that cannot be an intent (for example one that reads pixels). Answer that here, not in the frontend.
