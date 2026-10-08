# Domain docs

How the engineering skills read this repo's domain documentation. Single context.

## Before exploring, read these

- **`CONTEXT.md`** at the root: the glossary. It does not exist yet;
  `/domain-modeling` creates it when a term is first resolved. Proceed without it until then.
- **`docs/decisions/`**: the ADRs, `NNNN-slug.md`. Read the ones that touch the area.
- **`specs/<nnn>-<name>/spec.md`**: a spec-driven feature records its decisions
  and rejected alternatives here instead of in an ADR.
- **`docs/PRODUCT.md`** for product truth and **`docs/ARCHITECTURE.md`** for the crate layout.

## Use the glossary's vocabulary

Name domain concepts in the terms `CONTEXT.md` defines. The words the
interface says (Flagged, Archive, Thread, Mailbox, Sync, Compose) are fixed by
`/ux-architect`. A concept missing from both is a gap to note for `/domain-modeling`.

## Flag ADR conflicts

When output contradicts an ADR or a spec's decision, say so explicitly:

> _Contradicts ADR 0037 (…), but worth reopening because…_

New ADRs go in `docs/decisions/` with the next number. A spec and an ADR are
not both written for one decision (`CLAUDE.md`, "The loop").
