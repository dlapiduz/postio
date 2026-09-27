# ADR 0043: The GTK both desktop apps share lives in `postio-widgets`

- **Status:** Accepted (2026-09-27), with `specs/007-postio-focus`. The
  maintainer's handoff asked for it: "The shared GTK crate is a boundary that
  outlives this feature, so it gets its own ADR."
- **Spec:** [`specs/007-postio-focus`](../../specs/007-postio-focus/spec.md)
  (FR-006 to FR-008; research R1). The spec carries the reasoning; this
  records the rule.
- **Related:** [ADR 0041](0041-one-app-opens-the-store-at-a-time.md) (every
  frontend reaches mail through `postio-client`),
  [ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md)
  (the renderer the message view is built on).
- **Decision:** **Postio has two desktop apps, the classic app and Postio
  Focus, and what both of them draw lives in one crate, `postio-widgets`. Both
  depend on it, and neither depends on the other.**

---

## The rule

**What lives in `postio-widgets`:** the GTK that more than one desktop app
draws, and the presenters that join it to `postio-client`. That means:

- the message view on the reading renderer;
- the composer;
- the keycap, key-hint, chip, action-bar, notice and toast widgets, and the
  pickers;
- the list model over `postio-ui`'s window;
- the presenters for composing, reading, configuration and credentials.

**What does not live there:**

- **Anything only one app draws.** It stays in that app's crate: the classic
  shell, sidebar, reading pane and conversation rail in `postio-gtk`, and
  Focus's window, rows and dialogs in `postio-focus`.
- **Anything toolkit-free.** Logic both apps share without GTK belongs in
  `postio-ui` or `postio-core`, where a unit test runs in milliseconds.
- **Anything that opens the store or speaks a protocol.** The presenters reach
  mail through `postio-client`, as every frontend does (ADR 0041).

**Who depends on what:**

```text
postio-app ──▶ postio-gtk ──┐
                            ├──▶ postio-widgets ──▶ postio-client, postio-ui,
postio-focus ───────────────┘                        postio-render, postio-core, …
```

- `postio-gtk` and `postio-app` may depend on `postio-widgets`, and so may
  `postio-focus`.
- `postio-focus` may not depend on `postio-gtk` or `postio-app`, and
  `postio-gtk` may not depend on `postio-focus`.
- `postio-widgets` may not depend on any of the three. It may not depend on
  the store engine (`turso`, `rusqlite`), the protocol (`io-imap`), or the
  crates that own the store (`postio-host`, `postio-session`,
  `postio-runtime`, `postio-storage`, `postio-sync`) either.

`scripts/checks/check-crate-boundaries.py` enforces every line above, against
the resolved dependency graph.

## Alternatives

- **Focus depends on `postio-gtk`.** One crate would then carry two apps'
  shells, and every classic-app change would be a Focus build. The handoff
  ruled it out.
- **`postio-gtk` becomes the shared crate,** and the classic shell moves to
  `postio-app`. This is the same boundary with the names swapped, reached by
  moving every classic file rather than the shared ones.
- **Each app keeps its own copy.** Two readers and two composers drift within a
  release, which is the reason constitution II gives for one registry.

## Revisit when

A third GTK app appears. The rule would then say "every desktop app" rather
than "both".
