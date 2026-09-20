# Archived documents

Whole documents that no longer describe the system and are kept for the
record. Each says at the top why it is here. The dated engineering-notes
entries have their own archive under [`docs/notes/archive/`](../notes/archive/),
listed at the end of [`docs/engineering-notes.md`](../engineering-notes.md);
architecture decisions are never archived — a superseded ADR is amended in
place, see [`docs/decisions/README.md`](../decisions/README.md).

| Document | Why it is here |
|---|---|
| [`architecture-review-2026-08.md`](architecture-review-2026-08.md) | An outside review of the architecture from August 2026. Every one of its seven findings has since landed (`postio-session`, `postio-ui`, `postio-body`, ADR 0002, ADR 0013, the ten-crate boundary check) and every measurement in it is of a codebase that no longer exists — SQLCipher, rusqlite and FTS5 among them. `docs/ARCHITECTURE.md` records what remains open. |
