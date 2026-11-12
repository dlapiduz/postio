# Focus threads, step 7: replying to an older message and forwarding a thread

Spec 013, User Story 7 (T084). The `threads` demo seed was compared with
the maintainer's local `Design/focus-macos-threads/screens/09` and `10`,
which are not in the repository.

- **Mac.** The in-app snapshot was used, because `screencapture` returned
  black.
- **Linux.** The window was rendered on the verify box.

## What matches, on both platforms

- **09, replying to an older message.** `r` on message 4 of 6 raises an
  accent-tinted bar under the composer's title: "You're replying to
  message 4 of 6, not the latest", a second line naming the newer
  messages and saying the reply quotes and threads under the older one,
  and two actions.
  - Show them (`mod+shift+Down`) lists the newer messages read-only.
  - Reply to latest (`alt+e`) moves the reply onto message 6: the quote,
    In-Reply-To and References change, and what was typed stays.
- **10, forwarding a thread.**
  - `f` in a thread of two or more opens the composer with What to
    forward: three cards with their number keys (This message, Whole
    thread, As attachments).
  - `2` builds one readable message, oldest first, each part headed by
    sender and date, with every attachment once. The subject reads
    "Fwd: Harbor API draft v3 (6 messages)".
  - `3` attaches each message as an `.eml`.
  - Thread ▾'s "Forward the whole thread" opens on Whole thread.
  - When a message's body, source or attachment bytes are not on this
    machine, the chooser says so, stays on This, and fetches nothing
    (FR-028).

## Differences, and why

| Difference | Platform | Why |
|---|---|---|
| The Mac's screen 10 shows the chooser on This | Mac | It was shot before the threads seed stored its attachment's bytes, so Whole thread was rightly refused. The seed is fixed (`5bd8c7035`) |
| `1`/`2`/`3` choose a card while To is empty (GTK), or before the composer's own keys (Mac) | both, differently | Both should mean "while the chooser holds the keyboard". T085(j) |
| `mod+shift+q` does not fold the forwarded messages | GTK | The composer has no way to fold quoted rows yet. T085(k) |
| The composer shows a From row; the folded quote sits at the window's foot | Mac | The composer's existing layout, unchanged by this spec |
| Forwarded rows read "n · Who · 2026-…"; the design reads "Mon 21 Sep" | Mac | The row uses the ISO date; to switch to the reader's date words |
| Reply to latest leaves To and Cc as they are | both | The person may have edited them; the engine changes only the quote and the threading headers |
