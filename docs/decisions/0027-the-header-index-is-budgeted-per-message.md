# ADR 0027 — The header index is budgeted per message, not against the body index

- **Status:** Accepted (2026-09-13)
- **Decision:** `message_headers` plus `idx_message_headers_name` stay under 5 KiB per message, weighed as a file delta on the heavy fixture in `index_suite/header_index_size.rs` (4,218 B measured). ADR 0025's two caps do not move.

## Rules
- The size gate is the per-message ceiling, counting the table and the index together. It runs in the default suite.
- The ratio against the body index is printed as an observation and asserts nothing.
- `no_message_may_contribute_more_than_the_two_caps_allow` holds the caps exactly (512-byte values, 64 rows). `search_statement_budget.rs` holds the read path.
- The header index is fixed overhead against `[storage] max_bytes` and is not evictable. It rebuilds from `body_headers` with no network.
- Dropping the index to save space must say so in the UI (ADR 0025 Q5: never silently answer "no such mail").
- The caps have been priced: 256 saves about 27% and loses the deep substrings of `X-Spam-Report`. Do not reopen them to pass the gate.
- Header rows are bounded per message (at most 64 x 512 B), which is what makes the content duplication acceptable. They are not "metadata scale": about seventeen times the metadata half of the index.
- A change to either cap bumps `HEADERS_SCHEMA_VERSION`. Changes that do not move the caps do not.

## Why
- The body index holds no text, so a ratio against it measures how well bodies compress, not the header policy. It moved from 64% to 636% across fixtures with the policy unchanged, and fails hardest on mailboxes of short mail.
- Per-message bytes is invariant to mailbox size, controlled by the caps and multiplies straight into disk.
- Cost is intrinsic: substring-matching values needs the values on disk. Interning saves about a third; post-filtering in Rust reads the mailbox.
- Rejected: lower caps until the ratio passes (converges on an allowlist), a 650% ratio gate, a share of the whole file, a share of `body_headers` (no per-column instrument), no gate.

## Revisit when
- A per-column size measurement becomes cheap.
- `max_bytes` forces evicting the header index.
