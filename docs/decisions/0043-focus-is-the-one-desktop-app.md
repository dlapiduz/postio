# ADR 0043: Focus is the one desktop app, and the GTK it draws lives in `postio-widgets`

- **Status:** Accepted (2026-10-02), with `specs/007-postio-focus` (decision C27)
- **Decision:** Postio's GTK interface is the Focus design in `postio-gtk` (binary `postio`, app id `dev.postio.Postio`). There is no second GTK app. What it draws outside its own window lives in `postio-widgets`, which depends on no interface.

## Rules
- A fix goes in the app or a shared crate: `postio-widgets` for GTK, `postio-ui` or `postio-core` for toolkit-free logic, the engine for the rest. Use the lowest layer that can hold it. Put it in `postio-gtk` only if only the app's own window uses it.
- The design rules are Focus's (C25 and C26: system font, system accent).
- A new registry command is offered to Focus or to every app. Nothing is three-pane only unless macOS needs it.
- `postio-widgets` holds: the message view on the reading renderer, the composer, keycap, key-hint, action-bar, notice, toast widgets and pickers, the list model over `postio-ui`'s window, presenters for composing, reading, configuration and credentials, the settings window, the startup timeline, the design-token build and the icons.
- The app's window, rows and dialogs stay in `postio-gtk`. Toolkit-free code goes in `postio-ui` or `postio-core`. Anything that opens the store or speaks a protocol is reached through `postio-client` (ADR 0041).
- Dependencies: `postio-gtk` may depend on `postio-widgets` and runs `postio-host` in-process, but its own code may not depend on `turso`, `rusqlite` or `io-imap`. `postio-widgets` may not depend on `postio-gtk`, the store engine, the protocol, or `postio-host`, `postio-session`, `postio-runtime`, `postio-storage` or `postio-sync`. `check-crate-boundaries.py` enforces this.
- Classic-app capabilities and where they live now: `specs/007-postio-focus/classic-parity.md`.

## Why
- Two apps meant every fix twice or reaching only one, and a second design language. One app with two modes keeps both surface trees in one binary. Waiting for parity means two apps as long as parity takes.
- Folding `postio-widgets` into the app loses the window-free test seam and the boundary that keeps presenters on `postio-client`.

## Revisit when
- A second GTK app is proposed.
