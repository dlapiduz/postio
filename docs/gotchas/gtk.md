# Gotchas: GTK

Read before touching widgets, CSS or key handling. `/gtk-design` has the rest.

- **`connect_*` on a process-wide object outlives your widget.** For `adw::StyleManager::default()`, the display, settings or application, the closure keeps its strong references for the process. Disconnect when the last clone goes.
- **`set_content` on an `adw::Window` gives no titlebar.** Provide a `ToolbarView` with a `HeaderBar`. Widget tests cannot see it; render with `shot` or a storyboard.
- **To move a surface, reparent it, then restore focus.** Unparent from the real parent. In `close-request` call `destroy()`, not `close()`, which re-emits.
- **A Clamp cannot override a child's minimum width.** Unbroken addresses in a horizontal box sum their minimums and widen the column and dialog. Wrap recipient fields; prove the allocated size with long bare addresses.
- **A key controller on a dialog never beats the window's.** `keys_under_dialogs` gives every dialog the window's capture handler, so `#` or `e` first acts on the row behind. A dialog that wants a verb's key answers it in `FocusWindow::handle_key`; keep offers non-modal and give their action a registry command (`clear_list_mail`, #1952, #1953).
- **"Open" and "has the keyboard" are separate facts.** A surface left open while focus is elsewhere pins the resolver's context and drops bare keys.
- **Cursor, selection and activation are three facts.** Say which a surface follows. A verb with an empty selection acts on the cursor row; `SingleSelection` autoselects row 0, which is not the user landing there. Push state to `AppState` via `postio_core::aim::mirror`, not a signal.
- **Changes the app makes for the user stay off the undo stack, with no toast** (`Recording::Incidental`).
- **`GtkListView` builds about 205 rows ahead.** Use 1,000 or more test rows to tell recycling from none.
- **An app-priority CSS provider resolves `prefers-color-scheme` against its own setting.** `postio_gtk::style::install` keeps it in step with `AdwStyleManager`.
- **A second window's keyboard is the controller's fact, not `is_active`'s.** The popped-out Calendar reports `Input::WindowFocused` when it becomes active, and its keys are forwarded to the main window's one resolver; under a headless compositor `is_active` can lag, so tests and observations read `Feed::focused_window`. The storyboard runner films the main window only: a second window shows in the observation's `app.focus.*` fields, not in the frames.
