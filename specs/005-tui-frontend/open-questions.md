# Open questions for the maintainer

The branch took a defensible default for each question below, so nothing is
blocked on it. Each is written as what was chosen and what the alternative
is, so each can be settled in a line at review.

## The daemon

1. **Auto-reconnect.** When the daemon goes away mid-session, both frontends
   say so and reconnect only when asked: `R` in the terminal, "Try again" on
   the desktop. The alternative is reconnecting on their own, with backoff.
2. **Work made while disconnected.** Commands and draft saves made while the
   daemon is gone fail and are not replayed.
   - The terminal keeps an open draft's text, and will not close a draft
     with content until it has been saved again.
   - The alternative is queueing the work and replaying it on reconnect.
   - A send the daemon had already queued is in its store either way, and
     the next daemon carries it on.
3. **Wording when the daemon stops.** The desktop reuses the `Unavailable`
   screen, whose title still reads "Postio cannot open your mail", with the
   sentence "Postio's background service stopped…" beneath it. It could
   have a title of its own.
4. **The grace period.** The daemon stays up 30 seconds after the last
   frontend leaves, so a quick restart of either frontend does not reopen
   the store. It could be shorter, longer, or configurable.

## Behaviour that changed shape

5. **Undo is per frontend.** `u` undoes the last thing done in the frontend
   it is pressed in, not the last thing done anywhere. Undoing another
   window's archive from the terminal would surprise the person at the other
   screen.
6. **The Flatpak store path.** Both packages now keep the store at
   `~/.local/share/postio`, so that they can share it. An existing desktop
   Flatpak's store under `~/.var/app` is not moved; it resyncs. This follows
   the no-backwards-compatibility rule, but it is a visible resync for anyone
   with the Flatpak installed.
7. **The plain-text part of a Markdown message** is the Markdown source
   itself, sent as fixed text with its line breaks kept (no
   `format=flowed`). The alternative is re-wrapping it as flowed text.

## Keys

8. **Opening a link from the keyboard in the terminal.** A click shows
   where a link goes, and a second click opens it. There is no key for it
   yet. Numbered links with a follow-by-number command is the common
   terminal pattern, but it adds a registry command, so it is left for you
   to decide.
9. **Two registry commands the desktop only acknowledges.** `edit_externally`
   (`Alt+E`) and `toggle_preview` (`Alt+P`) exist for the terminal's
   composer. The desktop answers both with a status line rather than a
   behaviour. They could be given desktop meanings, or scoped to the terminal
   in the registry.

## The ADR

10. **ADR 0041 is Proposed** (T097). Accepting it at review makes one store
    owner the rule for every future frontend.
