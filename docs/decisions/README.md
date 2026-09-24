# Architecture decision records

One file per decision, numbered in the order they were made. An ADR is
never deleted or moved when it is overtaken: the correction is written
beside the text it corrects — a `> **Amended …**` blockquote at the affected
section, or a clause on the status line — so the reasoning stays readable
and the reader sees what changed and why. The table says where each
decision stands as of 0.4.0 (2026-09-14).

| ADR | Decision | Where it stands |
|---|---|---|
| [0001](0001-imap-library.md) | IMAP library: `io-imap` | Built (`postio-account`) |
| [0002](0002-extensible-command-vocabulary.md) | An extensible command vocabulary | Built; no extension registers yet |
| [0003](0003-rich-text-compose.md) | Rich-text (HTML) compose | Built |
| [0004](0004-composer-document-model.md) | The composer's document, and where it lives | Built (`postio-body`) |
| [0005](0005-multiple-accounts.md) | Multiple accounts and the unified inbox | Built |
| [0006](0006-oauth-and-provider-presets.md) | OAuth 2, and what "providers are data" has to mean | Built |
| [0007](0007-address-book.md) | The address book: one table, two provenances | Built, except vCard import/export and a management surface |
| [0008](0008-filters-and-rules.md) | Filters and rules: one language, two evaluators | Saved searches built; the rules engine is on `feature/rules`, not on `main` |
| [0009](0009-ai-subsystem.md) | The AI subsystem | Not built; deliberately after the fundamentals |
| [0010](0010-mcp-surface.md) | Exposing Postio over MCP | Not built; its prerequisites (`postio-session`, the event hub) are |
| [0011](0011-docs-site.md) | The docs site | Built |
| [0012](0012-add-account-and-orientation.md) | Adding a second account, and orienting a first-time user | Built |
| [0013](0013-event-fanout.md) | Event fan-out: a hub between producers and subscribers | Built |
| [0014](0014-encryption-at-rest.md) | The local store encrypts itself | Built; the mechanism is ADR 0038's, the threat model is this one's |
| [0015](0015-threaded-list.md) | One row per thread, and the conversation pane | Built |
| [0016](0016-full-mailbox-backfill-by-default.md) | Full-mailbox backfill by default, folders optionally excluded | Built |
| [0017](0017-backfill-cost-attachments-memory-disk-encryption.md) | What "download everything" costs, and the four axes that pay for it | Built; the storage axes are amended for the Turso engine |
| [0018](0018-jmap-and-gmail-backends.md) | JMAP and Gmail REST backends, on Pimalaya crates | Built |
| [0019](0019-macos-frontend.md) | A native macOS frontend over `postio-session` | Built as a read-only slice; not released |
| [0020](0020-where-message-bodies-live.md) | Message bodies live in the store; the blob store keeps attachments | Built; per-row zstd, no dictionary (see 0038) |
| [0021](0021-exactly-once-send.md) | Sending is at-most-once | Built |
| [0022](0022-extensions-contribute-rows-not-pixels.md) | Extensions contribute table rows, not pixels | Decided for the part that is forced; nothing registers yet |
| [0023](0023-reader-fonts-are-served-not-inlined.md) | The reader's fonts are served over a scheme, not inlined | Built |
| [0024](0024-layout-intent-and-constraint.md) | Layout intent is stored; the viewport's constraint is applied | Built |
| [0025](0025-arbitrary-headers-are-indexed-rows.md) | Arbitrary headers are stored on the row and indexed as rows | Built |
| [0026](0026-a-saga-carries-its-own-coordinates.md) | A saga's remove phase carries its own coordinates | Built |
| [0027](0027-the-header-index-is-budgeted-per-message.md) | The header index is budgeted per message | Built |
| [0028](0028-a-rule-runs-the-same-verb-a-keystroke-does.md) | A rule runs the same verb a keystroke does | Decided; waits on the rules engine |
| [0029](0029-one-control-vocabulary.md) | One control vocabulary | Built |
| [0030](0030-a-rule-stages-where-it-can-be-carried-out.md) | A rule stages where it can be answered and carried out | Decided; waits on the rules engine |
| [0031](0031-the-settings-window-is-one-model-two-frames.md) | The settings window is one model in two frames | Built |
| [0032](0032-the-conversation-is-one-document.md) | The conversation is one document | Built; its screen-reader gate is still open |
| [0033](0033-a-reply-quotes-what-the-reader-shows.md) | A reply quotes what the reader shows | Built |
| [0034](0034-one-composer-in-the-pane-many-in-windows.md) | One composer in the pane, many in windows | Built |
| [0035](0035-mailbox-roles-are-mapped-per-account.md) | Mailbox roles are mapped per account | Built |
| [0036](0036-a-sidebar-row-is-a-folder-or-a-view.md) | A sidebar row is a folder or a view | Built |
| [0037](0037-a-misspelling-is-answered-with-a-suggestion.md) | A misspelling is answered with a suggestion | Built |
| [0038](0038-the-store-is-turso-not-sqlcipher.md) | The store is Turso, and ADR 0014 keeps its threat model | Built |
| [0039](0039-the-composer-is-a-native-surface-over-the-document.md) | The composer is a native surface over the document | Decided; supersedes 0003 Q2, nothing built yet |
| [0040](0040-the-store-keeps-few-connections-maintains-its-counts-and-budgets-its-index.md) | The store keeps few connections, maintains its counts, and budgets its index | Proposed; the connection half (§1) landed with #1602, the rest awaits the maintainer |
| [0041](0041-one-process-owns-the-store.md) | One process owns the store; frontends are its clients | Proposed with `specs/005-tui-frontend`; nothing built yet |

## Writing one

Copy the shape of a recent one: a status line on line 3, the context, the
questions the decision answers, the alternatives rejected and why, the
consequences, and what would falsify it. Number it next in sequence. When
a later decision changes an earlier one, amend the earlier one in place and
say which ADR did it; do not rewrite the history it records.
