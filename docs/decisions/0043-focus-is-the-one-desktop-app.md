# ADR 0043: Focus is the one desktop app, and the GTK it draws lives in `postio-widgets`

- **Status:** Accepted (2026-10-02), with `specs/007-postio-focus` (decision
  C27; tasks T229-T231). The classic app is being retired; its removal waits
  for the maintainer.
- **Spec:** [`specs/007-postio-focus`](../../specs/007-postio-focus/spec.md)
  (FR-006 to FR-008; research R1).
  [`classic-parity.md`](../../specs/007-postio-focus/classic-parity.md) holds
  what the classic app does that Focus does not yet, and the retirement's
  order.
- **Related:** [ADR 0041](0041-one-app-opens-the-store-at-a-time.md) (every
  frontend reaches mail through `postio-client`),
  [ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md)
  (the renderer the message view is built on),
  [ADR 0031](0031-the-settings-window-is-one-model-two-frames.md) (the
  settings model is shared, the frame is each app's).
- **Decision:** **Postio has one desktop app, Postio Focus (`postio-focus`).
  The classic three-pane app -- `postio-gtk` and the `postio` binary in
  `postio-app` -- is retired: it is kept building and secure until it is
  removed, and gets no new work. What Focus draws, other than its own window,
  lives in `postio-widgets`, which depends on no app.**

---

## What other work obeys until the classic app is removed

1. **No new feature work in `postio-gtk` or in `postio-app`'s GUI.** What is
   allowed there:
   - keeping it compiling and its tests green as shared crates change;
   - security fixes;
   - moving code out of it, as the retirement plan orders.
2. **A fix goes in Focus or in a shared crate**: `postio-widgets` for GTK,
   `postio-ui` or `postio-core` for logic without a toolkit, the engine for
   the rest. If the classic app has the same bug, a fix in a shared crate
   reaches it too. A fix only `postio-gtk` could hold is not written.
3. **A capability Focus lacks is built in Focus**, from the gaps in
   `classic-parity.md`, never by extending the classic app. Code that moves
   out of `postio-gtk` goes to the lowest layer that can hold it, not into
   `postio-focus`, unless only Focus's own window uses it.
4. **The design rules are Focus's.** The spec's decisions C25 and C26 (the
   system font and the system accent) govern new surfaces. The classic app's
   PLATE tokens are not extended.
5. **A new registry command is offered to Focus**, or to every app. Nothing
   new is three-pane only unless the terminal needs it.
6. **Nothing of the classic app is deleted** until every gap in
   `classic-parity.md` is closed or the maintainer has accepted it, and the
   maintainer has approved the removal.

## Where the GTK lives

**What lives in `postio-widgets`:** the GTK that Focus draws outside its own
window, and the presenters that join it to `postio-client`. That means:

- the message view on the reading renderer;
- the composer;
- the keycap, key-hint, chip, action-bar, notice and toast widgets, and the
  pickers;
- the list model over `postio-ui`'s window;
- the presenters for composing, reading, configuration and credentials;
- whatever the retirement moves out of `postio-gtk` and `postio-app` for
  Focus to use: the settings window, the startup timeline, the design-token
  build and the app's icons among them.

**What does not live there:**

- **Focus's window**, rows and dialogs. They stay in `postio-focus`.
- **Anything toolkit-free.** It belongs in `postio-ui` or `postio-core`, where
  a unit test runs in milliseconds.
- **Anything that opens the store or speaks a protocol.** The presenters reach
  mail through `postio-client`, as every frontend does (ADR 0041).

**Who depends on what**, until the removal:

```text
postio-app ──▶ postio-gtk ──┐
                            ├──▶ postio-widgets ──▶ postio-client, postio-ui,
postio-focus ───────────────┘                        postio-render, postio-core, …
```

- `postio-gtk`, `postio-app` and `postio-focus` may depend on
  `postio-widgets`.
- `postio-focus` may not depend on `postio-gtk` or `postio-app`, and
  `postio-gtk` may not depend on `postio-focus`.
- `postio-widgets` may not depend on any of the three. It may not depend on
  the store engine (`turso`, `rusqlite`), the protocol (`io-imap`), or the
  crates that own the store (`postio-host`, `postio-session`,
  `postio-runtime`, `postio-storage`, `postio-sync`) either.

After the removal, the left-hand branch goes and the rest of the rule stands.

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
  apps for as long as parity takes. The freeze is what stops a fix from
  landing twice in the meantime.
- **Fold `postio-widgets` into `postio-focus` once only one app is left.**
  The crate is where a component is tested without the window, and its
  boundary is what keeps the presenters on `postio-client`. One app depending
  on it is reason enough to keep it.

## Revisit when

- The maintainer approves the removal. The rules under "until the classic
  app is removed" then go, and the dependency diagram loses `postio-gtk` and
  `postio-app`'s GUI.
- A second GTK app is proposed. This ADR is the argument it has to answer.
