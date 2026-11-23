# ADR 0034 — One composer in the pane, many in windows

- **Status:** Accepted (2026-09-11). In Focus, "the pane" is the compose dialog or the open message's place beside the list; a draft may detach to its own window (spec 007 FR-050).
- **Decision:** The composer is not a singleton. The pane holds at most one; every other open draft is a window of its own; a draft is open in exactly one of them. Detaching is lossless because the draft is the record (ADR 0004), not because a widget is never rebuilt.

## Rules
- Starting a draft while the pane holds one moves the pane's draft into a detached window. It never refuses, discards or asks (FR-011).
- Attaching into an occupied pane pushes the current draft into a window and the arriving one takes the pane. It is not a swap and not a refusal.
- Moving a draft between surfaces is save then `resume`. `resume` carries text, formatting, recipients and attachments. Undo history is lost and the caret returns to the start of the body. `detach` of the one composition still reparents.
- A command acts on one composer only. `connect_command` carries which surface resolved the key, and a composer acts when it is that surface. Never infer from window focus, because windows are not reliably active under the headless runner. Without this, Send sends every open draft. Do not build the multi-composer state without the routing.
- `Compose` pressed in a detached window opens the new draft in the pane. Palette commands arrive with the main window active and belong to the pane's composer.
- A new composer replaces the pane's registration for its occupant kind (`register_reader_occupant`) and its `compose` `GAction`. The old composer hands the pane back before the next takes it (#502).
- Every open composer says which draft it holds, and a detached window is identifiable from the window list without focus (FR-014). Asking for an already-open draft brings its surface forward (FR-013).
- `focus_suite::compose_detach` covers detach and attach in the running window.

## Why
- Refusing is quiet: the key appears to do nothing. Discarding is unacceptable.
- Rejected: one composition total (the maintainer's choice, 2026-09-10); a dialog asking (the answer is always yes); tabs in the pane (a new navigation level inside a borrowed surface, and windows are the level the desktop provides).
- Cost: each composer is a `WebView`, so N open drafts are N engine processes (#957). The multiplier is open drafts if the composer ever grows another `WebView`.

## Revisit when
- Composer undo history must survive a move between surfaces.
