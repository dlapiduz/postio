# 0046 — Content identity is distinct from mailbox identity

Status: Accepted (2026-10-05); amended 2026-11-22 (#1526)

**Decision:** `messages` stays the occurrence and the bounded list projection. A separate `message_contents` row owns decoded text, HTML and header bytes. Only a backend that guarantees an immutable account-wide content id opts into sharing. Search alone also collapses exact copies.

## Rules
- `messages.id` identifies a mailbox occurrence and is the target of flags, pending operations, snoozes and remote mutations. Content sharing preserves location IDs, mailbox addresses, flags and pending operations.
- A backend supplies `ContentIdentity { namespace, key }` only when it guarantees the key names immutable content across mailboxes. Storage scopes it to the local account. No identity allocates independent content, even when remote coordinates or RFC IDs match.
- Gmail message IDs and JMAP Email IDs opt in, under different namespaces. IMAP, Maildir and mocks do not. A backend's addressing ID never opts in implicitly. Adapters own the guarantee through the `MailBackend` seam.
- Search documents belong to content. Search picks a qualifying occurrence after account, folder and flag constraints, so an Archive match returns an Archive mutation target.
- Envelope, availability and MIME metadata stay as small projections on occurrences. Reusing content reuses its MIME parts and availability. Removing a membership retains shared content. Removing the last collects it.
- Migration gives each existing occurrence independent content and never infers identity from old RFC IDs or coordinates. Unknown schema fingerprints stay refused.
- Search collapses an occurrence into an exact copy: same account, same `rfc_message_id`, same non-zero size, same `Date`, same people including From (`people_ids`). The earliest qualifying occurrence stands for the copies. Counts and folder facets count it once, in every folder holding a copy. A copy stands only if it matched and has content of its own. Storage, flags, bodies and mutation targets are untouched.
- Tests prove sharing and non-sharing, late membership reuse, last-membership cleanup, scoped search and retained mutation targets without a live server. Read paths keep indexed, bounded lookups and mailbox paging guarantees.

## Why
- JMAP and Gmail expose one immutable message in several folders. IMAP UIDs are scoped to mailbox and UIDVALIDITY, and an RFC Message-ID can be reused or forged.
- Rejected: Message-ID search collapse alone (cannot establish identity, saves no storage); sharing bodies between IMAP copies (matching envelopes do not prove identical bodies); inferring identity from `remote_id`; replacing occurrence IDs with content IDs (ambiguous flags and moves); removing list projections (adds joins to the windowed list).
