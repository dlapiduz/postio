# `connect_action` cannot see `j` (2026-09-04, #288)

The first-run keyboard orientation retires on "the user's first command"
(ADR 0012 Q6), and the obvious seam for that is `Window::connect_action` --
its own doc comment says it is called with *every* invocation, and it is the
seam a command bus subscribes to. It never fires for `j`.

`Window::act` is the one door every gesture comes through, and it forks:

```
act(command)
  -> handled_here(id)?  yes -> return          <- cursor moves, overlays close
  -> deliver(command)          <- connect_command and connect_action handlers
```

So the two public seams see the commands the window passes **out** to the
bus, not the ones it answers itself. `NextMessage`, `PrevMessage`,
`SelectAll`, `Back`, every `ScrollReader*` and the whole parts panel stop at
`handled_here`. A subscriber wanting "did the user run a command" therefore
sees archive and flag and never sees the cursor move -- which for a feature
about teaching `j`/`k` is precisely backwards, and it looks like nothing at
all rather than like an error.

`act` is not the fix either: it also carries the mouse's invocations, and
`MarkReadOnDwell` fires from hovering a message. ADR 0012 Q6 is explicit that
a click is not evidence of anything.

The seam that means "the user used the command system" is **`run_action`**,
which is private and has exactly two callers: `resolve_key` and the command
palette's `connect_command`. Keyboard and palette, never the mouse. That is
where the orientation retires itself, inside `postio-gtk` -- the only layer
that can tell those apart -- and `postio-app` subscribes to the strip's own
`connect_retired` to write the flag.

The general shape: **"every command" in this window means three different
sets**, and which one a feature wants depends on whether it is about the
mail (`connect_action`), about any gesture (`act`), or about the person
having used the keyboard (`run_action`).
