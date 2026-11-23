# ADR 0025 — Arbitrary headers are stored on the row and indexed as rows, not as text

- **Status:** Accepted (2026-09-03); amended 2026-10-08 by [ADR 0046](0046-content-identity-is-distinct-from-mailbox-identity.md)
- **Decision:** The header block is persisted whole; a normalized `message_headers(message_id, name, value, ordinal)` table in `postio-index` is what `header:` matches; every header is indexed, bounded by structural caps and never by a name list. `header:` is a body-class fact.

## Rules
- Header blocks and derived header rows belong to `message_contents` and are keyed by content (ADR 0046). Sharing needs an account-scoped immutable backend identity, never an RFC Message-ID.
- The block is stored whole in `body_headers`, at most 256 KiB (longer is truncated and marked). It is the rebuild source, so a policy change never needs the network. The raw blob is never the source: it is evicted first.
- `message_headers` is derived index data in `postio-index`, versioned by a `headers` row in `search_schema`. A version bump drops it and a background local pass refills it.
- Caps: values truncated to 512 bytes; at most 64 rows per message, in wire order. No allowlist or denylist of header names. The budget is ADR 0027's. If it is missed, tune the caps.
- One normalization (lowercase name, unfold, RFC 2047 decode, collapse whitespace, truncate) lives in `postio-model::headers` and both evaluators use it.
- `header:name` matches presence. `header:name=value` matches a case-insensitive substring. Split at the first `=`. The name matches exactly; any occurrence matching is a match; `header:x=` means presence.
- `Filter::Header { name, value }` is answered by both evaluators. `header:` classifies as `NEEDS_BODY` (ADR 0008 Q3), not `HEADERS_ONLY`.
- A header needed before the body earns a dedicated operator, column and `HEADER.FIELDS` fetch. `List-Unsubscribe`, `Precedence` and `Auto-Submitted` are promoted this way (`is:bulk`, `is:automated`), read via `postio_model::promoted`. NULL means unknown and is never overwritten by a write that does not know.
- Missing header rows fill from `body_headers`, then from the raw blob (written back), then by a backfill-lane fetch. Only the last touches the network.

## Why
- Tokenizers split `spf=pass` and `1.5.24`, the values people match. A bounded content table beats a full-text row that needs a side table to map to its message.
- A name list is maintained forever, makes unlisted names answer "no mail", and converges on provider-specific constants.
- Fetching an allowlist at sync time taxes every sync for a rare benefit.
- Rejected: a single full-text index over the block (cannot bind name to value), matching in Rust (reads the mailbox), a `[search] indexed_headers` setting, indexing only rule headers.

## Revisit when
- Measured header-index cost exceeds ADR 0027's ceiling even with tuned caps.
