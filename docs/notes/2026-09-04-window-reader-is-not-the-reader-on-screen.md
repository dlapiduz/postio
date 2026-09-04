# `Window::reader()` is not the reader on screen (2026-09-04, #1030)

A `shot` mode that called `window.reader().render(...)` produced a picture of
whatever the demo had already drawn — three times, with three different
theories about async loads and pump ordering. None of them was it.

`Window::reader()` returns the **single-message** reader. In a conversation
that widget is not the pane's occupant: `reader_showing()` prefers
`conversation().reader_for(focused())` and only falls back to `reader()` for a
folder row that is not a thread. The demo seed opens a conversation, so the
render was landing on a hidden widget and the visible one kept its content.

`reader_showing` is private, but both halves of it are public, so anything
outside the crate that wants the reader a person is looking at wants

```rust
window.conversation().focused()
    .and_then(|message| window.conversation().reader_for(message))
    .unwrap_or_else(|| window.reader())
```

The general shape: a widget's accessor on `Window` is the one **it** owns, not
necessarily the one the pane arbiter (#502) is currently showing. Rendering
into a hidden widget fails silently in exactly the way a screenshot cannot
reveal — the picture looks fine, it is just of something else.
