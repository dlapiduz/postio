# ADR 0045 — Focus's behaviour lives in `postio-focus`; frontends draw its intents

- **Status:** Accepted (2026-10-07), with `specs/009-focus-macos`
- **Spec:** [`specs/009-focus-macos`](../../specs/009-focus-macos/spec.md)
  (FR-002 to FR-005; research R1 to R4 carry the reasoning, and this ADR
  records the rule)
- **Related:**
  - [ADR 0043](0043-focus-is-the-one-desktop-app.md): Focus is the one
    desktop design, and its GTK lives in `postio-widgets`.
  - [ADR 0041](0041-one-app-opens-the-store-at-a-time.md): every frontend
    reaches mail through `postio-client`.
  - [ADR 0019](0019-macos-frontend.md): the Mac frontend over `postio-ffi`.
  - [ADR 0044](0044-every-frontend-is-observable-and-storyboarded.md):
    `observe()`.
- **Decision:** **What Focus does is decided once, in `postio-focus`, by a
  controller that does no I/O. A frontend turns key presses, engine events,
  client replies and facts about its own window into the controller's inputs,
  and draws the intents that come out. No rule about what a command does,
  which surface a key reaches, where the cursor goes, or what a surface
  holds lives in a frontend's window code.**

---

## The rule

1. **One controller.** `postio_focus::FocusController` owns:
   - the cursor, the selection and the current place;
   - the stack of open surfaces;
   - each surface's own state;
   - the Focus key resolver.

   It is `Send` and holds no toolkit object.
2. **Sans-IO.** The controller never awaits.
   - **What it emits:** `Effect::Show(Intent)` for the frontend to draw,
     `Effect::Ask(ticket, Request)` for the engine, and `Effect::Timer`.
   - **Requests.** One function, `postio_focus::perform(&Client, Request)`,
     maps a request to the client. The frontend runs it on its own executor
     and feeds the reply back with its ticket.
   - **Stale replies.** A reply for a stale generation changes nothing.
3. **Frontends draw.** `postio-gtk` and `postio-ffi`, and any later frontend,
   are drivers:
   - They resolve nothing themselves.
   - They apply intents in order.
   - They report facts the controller cannot see, such as the window size,
     a pointer placing the cursor, or a surface closed by its own button.

   A behaviour a frontend needs and the controller lacks is added to the
   controller, with its test, before the frontend draws it.
4. **Platform differences are policy, not forks.**
   - **Policy.** `Policy { platform, caps }` says what differs: geometry
     (spec 009 M1), whether secondary surfaces stack or replace each other
     (M4), and whether reading beside the list exists.
   - **Rules.** The rules are the same rules.

## Boundary

`postio-focus` may depend on `postio-ui`, `postio-client`, `postio-core`,
`postio-model`, `postio-config` and `postio-search`. It may not depend on:

- a toolkit: GTK, GDK, libadwaita, webkit, their `-sys` crates, or AppKit by
  any route;
- the store or the protocol: `turso*`, `rusqlite`, `libsqlite3-sys`,
  `io-imap`;
- the crates that own them: `postio-host`, `postio-session`,
  `postio-storage`, `postio-runtime`;
- the frontends: `postio-widgets`, `postio-gtk`, `uniffi`.

It may not name `tokio`, `glib` or `async-std` as a direct dependency.
`scripts/checks/check-crate-boundaries.py` enforces this list.

`postio-ui` keeps its role: pure words and decisions (`focus_row`,
`focus_dialog`, `focus_state`, `selection::Selector`, the key resolver), with
no stateful owner and no client.

## Alternatives

- **A module in `postio-ui`.** `postio-client` already depends on `postio-ui`
  for onboarding and recipient types, so the controller there could not name
  a client type without a cycle.
- **The controller owns a `Client` and spawns.** That ties it to one executor:
  GTK's main loop is glib, and the FFI's is tokio. It also makes every rule
  an integration test.
- **Each frontend keeps its own rules**, as GTK's window did until spec 009.
  The Mac would have rebuilt them in Swift. The FFI's own copy had already
  drifted: `Back` cleared only a selection, and the anchor was a row index
  where GTK's is a message.
- **Export GTK's window over the FFI.** Its types cannot cross the boundary.

## Revisit when

- A frontend needs a behaviour that cannot be expressed as an intent: for
  example, one that must read pixels to decide. That is an argument to answer
  here, not a reason to put the rule in the frontend.
- The terminal adopts the controller. Its `Input`/`Effect` machine
  (`postio-tui/src/app.rs`) is the same pattern, and folding it in should
  leave this rule unchanged.
