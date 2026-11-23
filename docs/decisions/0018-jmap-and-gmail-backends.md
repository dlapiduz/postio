# ADR 0018 — JMAP and Gmail REST backends, on Pimalaya crates

- **Status:** Accepted (2026-08-27), maintainer-directed
- **Decision:** Postio has two more `MailBackend` implementations, JMAP via `io-jmap` and Gmail REST via `io-gmail`. A `backend` preference list on the preset row selects one per account. IMAP and SMTP stay the default and the reference.

## Rules
- `postio-jmap` and `postio-gmail` depend on their Pimalaya wire crate and on `postio-account` with `default-features = false`, implement the trait, and leave everything above the composition root unchanged. `check-crate-boundaries.py` bars GTK, SQL and `io-imap` types from both.
- The backend is a preset field (`backend = ["imap"]` by default), never a branch on a vendor name.
- Message identity is `messages.remote_id TEXT`, an opaque `RemoteId`. `uid` and `uid_validity` belong to the IMAP adapter, which keeps generations and `needs_resync` behind the seam.
- Delta sync emulates first: adapters store their cursor (`Email/changes` state, Gmail `historyId`) where the sync-state row keeps its high-water mark. A native `SyncStrategy` seam waits for a measured cost.
- Gmail labels map system labels to mailbox roles and the rest to `message_labels`. Archive means remove the INBOX label, one operation per verb.
- Both authenticate with ADR 0006's `TokenSource`. Gmail's restricted scopes work with own-client credentials or a broker; the Gmail row advertises `["imap", "gmail"]` until the verified client exists, then flips preference (a data change).
- Sending stays on SMTP for both. Native submission (`EmailSubmission`, `users.messages.send`) is a later per-backend slice where SMTP is worse.
- Pin minor versions of 0.x crates; the `MailBackend` mock-driven tests are the compatibility net.

## Why
- Fastmail's native protocol is JMAP (`/changes` over CONDSTORE emulation, push without IDLE); Gmail's long arc bends away from raw IMAP for third parties.
- The trait is the abstraction (ADR 0001); a second layer would put every verb across two seams. Waiting for 1.0 is how the Pimalaya-first gap arose.

## Revisit when
- Emulated delta sync's cost is measured, or SMTP is the worse path for a provider.
