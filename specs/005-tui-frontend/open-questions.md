# Questions for the maintainer, answered

The branch took a default for each question below and asked the maintainer to
settle it at review. All five were answered on 2026-09-25; this file keeps
the answers beside the questions, so the reason for each behaviour is in the
repository rather than in a conversation.

One question was settled earlier: whether the two apps could run at once.
They cannot; the daemon that allowed it was removed on 2026-09-24 (spec
Clarifications, ADR 0041).

## Behaviour that changed shape

1. **The Flatpak store path.** Both packages keep the store at
   `~/.local/share/postio`, so that they can share it. An existing desktop
   Flatpak's store under `~/.var/app` is not moved; it resyncs.
   **Answer: kept.** There are no installs to protect yet (the
   no-backwards-compatibility rule).
2. **The plain-text part of a Markdown message** is the Markdown source
   itself, sent as fixed text with its line breaks kept (no
   `format=flowed`). **Answer: kept; no re-wrapping.**

## Keys

3. **Opening a link from the keyboard in the terminal.** A click shows where
   a link goes, and a second click opens it; there is no key.
   **Answer: left as it is for now.** No registry command is added.
4. **`edit_externally` (`Alt+E`) and `toggle_preview` (`Alt+P`)** exist for
   the terminal's composer. **Answer: scoped to the terminal.** They carry
   `Requirement::Terminal`, so only the terminal's palette and cheat sheet
   offer them and no menu lists them. The desktop still answers the key with
   one line saying why, as the registry requires of a bound key that cannot
   run.

## The ADR

5. **ADR 0041.** **Answer: accepted** (T097): one app opens the store at a
   time, each running the host inside it. It is the rule for every future
   frontend.
