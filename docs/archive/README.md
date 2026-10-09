# Archived documents

What no longer describes the system, kept for the record and for grep.
Nothing here is a rule; the live docs are `CLAUDE.md`, `docs/gotchas.md`,
`docs/notes/`, `docs/decisions/` and `specs/`.

- [`notes/`](notes) -- dated notes whose subject is gone or whose lesson is
  now in the code or a check.
- [`specs/`](specs) -- specs 001-006, for features that shipped (some of
  them in the three-pane app that Focus replaced).
- [`engineering-notes.md`](engineering-notes.md) -- the old lessons file;
  what still applies is in `docs/gotchas.md`.

| Document | Why it is here |
|---|---|
| [`engineering-notes.md`](engineering-notes.md) | The lessons file through 2026-10-07. Much of it describes the removed three-pane app; the traps that still apply moved to `docs/gotchas.md`. |
| [`architecture-manual.md`](architecture-manual.md) | A tutorial for a non-Rust reader, built on SQLCipher and an encrypted WAL; the store is Turso (ADR 0038). `docs/ARCHITECTURE.md` is current. |
| [`session-prompts.md`](session-prompts.md), [`session-prompts-backlog.md`](session-prompts-backlog.md) | Role prompts from early September; the skills replaced them. |
| [`architecture-review-2026-08.md`](architecture-review-2026-08.md) | An outside review of the architecture from August 2026. Every one of its seven findings has since landed (`postio-session`, `postio-ui`, `postio-body`, ADR 0002, ADR 0013, the ten-crate boundary check) and every measurement in it is of a codebase that no longer exists — SQLCipher, rusqlite and FTS5 among them. `docs/ARCHITECTURE.md` records what remains open. |
