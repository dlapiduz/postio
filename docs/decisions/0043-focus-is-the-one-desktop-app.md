# ADR 0043: Focus is the one desktop app, and the GTK it draws lives in `postio-widgets`

- **Status:** Accepted (2026-10-02), with `specs/007-postio-focus` (decision
  C27; tasks T229-T231). The classic app was removed in T256 (approved by
  the maintainer on 2026-10-02).
- **Spec:** [`specs/007-postio-focus`](../../specs/007-postio-focus/spec.md)
  (FR-006 to FR-008; research R1).
  [`classic-parity.md`](../../specs/007-postio-focus/classic-parity.md) says
  where each capability of the classic app lives now.
- **Related:** [ADR 0041](0041-one-app-opens-the-store-at-a-time.md) (every
  frontend reaches mail through `postio-client`),
  [ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md)
  (the renderer the message view is built on),
  [ADR 0031](0031-the-settings-window-is-one-model-two-frames.md) (the
  settings model is shared, the frame is each app's).
- **Decision:** **Postio has one desktop app, Focus, in the crate
  `postio-gtk`: named Postio, the binary `postio`, the app id
  `dev.postio.Postio`. What it draws, other than its own window, lives in
  `postio-widgets`, which depends on no app.**

---

## What other work obeys

1. **A fix goes in the app or in a shared crate**: `postio-widgets` for GTK,
   `postio-ui` or `postio-core` for logic without a toolkit, the engine for
   the rest. Code goes to the lowest layer that can hold it, not into
   `postio-gtk`, unless only the app's own window uses it.
2. **The design rules are Focus's.** The spec's decisions C25 and C26 (the
   system font and the system accent) govern every surface.
3. **A new registry command is offered to Focus**, or to every app. Nothing
   new is three-pane only unless macOS needs it.

## Where the GTK lives

**What lives in `postio-widgets`:** the GTK the app draws outside its own
window, and the presenters that join it to `postio-client`. That means:

- the message view on the reading renderer;
- the composer;
- the keycap, key-hint, action-bar, notice and toast widgets, and the
  pickers;
- the list model over `postio-ui`'s window;
- the presenters for composing, reading, configuration and credentials;
- the settings window, the startup timeline, the design-token build and the
  app's icons.

**What does not live there:**

- **The app's window**, rows and dialogs. They stay in `postio-gtk`.
- **Anything toolkit-free.** It belongs in `postio-ui` or `postio-core`, where
  a unit test runs in milliseconds.
- **Anything that opens the store or speaks a protocol.** The presenters reach
  mail through `postio-client`, as every frontend does (ADR 0041).

**Who depends on what:**

```text
postio-gtk ──▶ postio-widgets ──▶ postio-client, postio-ui,
                                  postio-render, postio-core, …
```

- `postio-gtk` may depend on `postio-widgets`. It opens the store in its own
  process, through `postio-host`, so the engine is in its graph; its own code
  does no SQL and speaks no protocol, so it may not depend on the store engine
  (`turso`, `rusqlite`) or the protocol (`io-imap`) directly.
- `postio-widgets` may not depend on `postio-gtk`. It may not depend on the
  store engine, the protocol, or the crates that own the store
  (`postio-host`, `postio-session`, `postio-runtime`, `postio-storage`,
  `postio-sync`) either.

`scripts/checks/check-crate-boundaries.py` enforces every line above, against
the resolved dependency graph.

## Alternatives

- **Two desktop apps.** Every fix had to be made twice or reached only one
  app. The store refusal said "Start a fresh store" in Focus and "Try again"
  in the classic app; the first-sync "inbox empty" needed a second pass for
  the classic app; key routing and the one close button exist only in Focus.
  Changes to shared widgets (icon buttons, the composer toolbar, the reader's
  stylesheet) rippled into a second design language that C25 and C26 had
  deliberately left behind. Two app ids complicated the package.
- **One app with two modes**, the three-pane layout as a mode of Focus. It
  keeps both surface trees and both rule sets in one binary, and adds a mode
  the person has to know they are in. The one thing the three-pane layout
  offered that Focus lacked, reading beside the list, comes back as Focus's
  own layout (T232).
- **Keep building both until Focus is at parity, then retire.** That is two
  apps for as long as parity takes. A freeze on the classic app kept a fix
  from landing twice until it was removed.
- **Fold `postio-widgets` into the app's crate now that only one app is left.**
  The crate is where a component is tested without the window, and its
  boundary is what keeps the presenters on `postio-client`. One app depending
  on it is reason enough to keep it.

## Revisit when

- A second GTK app is proposed. This ADR is the argument it has to answer.
