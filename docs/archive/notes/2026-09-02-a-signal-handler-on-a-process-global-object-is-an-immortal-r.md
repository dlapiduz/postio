# A signal handler on a process-global object is an immortal reference (2026-09-02, #794)

`Reader::with_allowlist` did this, and it looks entirely reasonable:

```rust
adw::StyleManager::default().connect_dark_notify({
    let view = view.clone();
    move |_| paint_ground(&view)
});
```

Repaint the pane when the colour scheme changes. The handler is never
disconnected, because nothing suggests it needs to be — but
`adw::StyleManager::default()` is a **process-global singleton**, so that
closure, and the strong `WebView` reference inside it, live as long as the
process. Every reader ever built leaks its view.

In the application that is invisible: there is one reader and it lives as long
as the window. In a **test binary** it is #794. A `WebView` owns a
`WebContext`, and a `WebContext` is a *WebProcess*. Twenty tests that each
stand up a reader leave twenty WebProcesses attached, and at `exit()` the UI
process tears its side of those connections down while they are still
running:

```
** (process:2): ERROR **: WebProcess didn't exit as expected after the UI
process connection was closed        (once per leaked view)
```

then SIGSEGV. Intermittent, because it is a race between exit handlers and
processes that should already be gone — which is what made it look like a
flake rather than a leak.

**The lesson is about where the reference lives, not about WebKit.**
`connect_*` on anything a widget does not own outlives the widget: the style
manager, the display, a settings object, an application. `Reader` is `Clone`
and every field is a handle, so the disconnect is an `Rc<DarkNotify>` whose
`Drop` fires when the last clone goes — a plain `impl Drop for Reader` would
unhook a reader that is still on screen the moment the first clone went out
of scope.

### Test the leak, not the crash

The crash is a race and **does not reproduce on this workstation at all**:
the binary that failed on CI passes 25 runs out of 25 here, with and without
the fix. Reproducing it was never going to be the test.

The leak underneath it is deterministic and takes milliseconds: hold a
`WeakRef` to the view, drop the reader, turn the main loop, require
`upgrade()` to be `None`. Before the fix, five of five survived. That is the
shape to reach for whenever a teardown crash is intermittent — the crash is
the symptom of something that is, on its own, perfectly reproducible.
