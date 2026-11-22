# ADR 0009 — The AI subsystem

- **Status:** Accepted (2026-08-24). Built: `postio-ai` as the client for a model the person runs on this computer, used by Focus. Not built: summarise, draft reply, semantic search, remote providers.
- **Decision:** `postio-ai` is an engine-rank crate that cannot send mail, structurally. It produces proposals, never actions. Semantic search is hybrid retrieval that re-ranks. Every provider call lands in the egress log, and a cloud provider stays off until a per-account, per-feature opt-in.

## Rules
- `postio-ai` has no `MailBackend` or SMTP in its dependency closure: it does not depend on `postio-account`, `postio-smtp`, `postio-sync` or `postio-runtime` (`check-crate-boundaries.py`). Its output is a `Proposal` (draft, summary, action items, ranked ids); the composition root turns it into a `Command` only when a human accepts.
- `AiProvider` exposes `locality() -> OnDevice | Remote`; the consent check, the status indicator and the egress log read it. The default provider is on-device (Ollama over local HTTP). Nothing is bundled. Consent is per `(account, feature, provider)`. With no reachable provider the AI commands are absent from the palette.
- Semantic search is a retrieval strategy, not an operator. `postio-search` does not change. Embeddings are computed lazily on backfill, only when enabled, stored `i8`-quantised in `message_embeddings`, and deleted when consent goes off. Full-text candidates are always merged, so a message without an embedding is still found. Scan brute force first; `search_budget.rs` gates any ANN index.
- Prompt injection: summarise and draft requests carry no tool definitions. Message content is fenced and labelled untrusted, never concatenated into instructions. Output is inserted as `Inline::Text`, so it cannot carry a link or remote image. A drafted reply opens in the composer, and the user's normal Send is the confirmation.
- The `untrusted-instructions` corpus fixture (category `prompt-injection`, shared with ADR 0010) must produce no command, no request beyond the one completion, and no live link.
- AI commands are ordinary registry commands in an `ai:` namespace. `destructive: true` with `Recovery::None` is rejected at registration.
- The egress log is the `egress_log` table (`subsystem` includes `'model'`): when, account, host and port, outcome. It records ids, counts and outcomes, never prompts. `check-no-silent-tracking.py` lists AI endpoints and requires a `POSTIO-CONSENT:` note for a new remote call.

## Why
- Mail is attacker-controlled text and an AI feature turns text into behaviour. A crate boundary is checked on every push; a "must ask first" rule is one refactor from untrue. A confirmation dialog can be routed around, a missing dependency cannot.
- A second retrieval system breaks the one-language rule (`ARCHITECTURE.md` §6) and the 100 ms budget. A `semantic:` operator would make a saved search mean different things.
- The default is local because the default must be the one that cannot leak. A bundled model ships weights Postio must keep current.

## Revisit when
- Brute-force vector scan misses the search budget at realistic mailbox sizes.
