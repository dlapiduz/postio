# Open questions for the maintainer

The branch took a defensible default for each question below, so nothing is
blocked on it. Each is written as what was chosen and what the alternative
is, so each can be settled in a line at review.

One question was settled while this list stood: whether the two apps could
run at once. They cannot; the daemon that allowed it was removed on
2026-09-24 (spec Clarifications, ADR 0041), and with it the questions about
reconnecting, work made while disconnected, the grace period and undo across
frontends.

## Behaviour that changed shape

1. **The Flatpak store path.** Both packages now keep the store at
   `~/.local/share/postio`, so that they can share it. An existing desktop
   Flatpak's store under `~/.var/app` is not moved; it resyncs. This follows
   the no-backwards-compatibility rule, but it is a visible resync for anyone
   with the Flatpak installed.
2. **The plain-text part of a Markdown message** is the Markdown source
   itself, sent as fixed text with its line breaks kept (no
   `format=flowed`). The alternative is re-wrapping it as flowed text.

## Keys

3. **Opening a link from the keyboard in the terminal.** A click shows
   where a link goes, and a second click opens it. There is no key for it
   yet. Numbered links with a follow-by-number command is the common
   terminal pattern, but it adds a registry command, so it is left for you
   to decide.
4. **Two registry commands the desktop only acknowledges.** `edit_externally`
   (`Alt+E`) and `toggle_preview` (`Alt+P`) exist for the terminal's
   composer. The desktop answers both with a status line rather than a
   behaviour. They could be given desktop meanings, or scoped to the terminal
   in the registry.

## The ADR

5. **ADR 0041 is Proposed** (T097), as revised on 2026-09-24: one app opens
   the store at a time, each running the host inside it. Accepting it at
   review makes that the rule for every future frontend.
