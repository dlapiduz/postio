//! Gmail REST [`MailBackend`] adapter over Pimalaya's `io-gmail` (#546,
//! ADR 0018 Q4/Q5).
//!
//! The third implementation of the seam ADR 0001 drew. Gmail's model is
//! labels, not folders, and this adapter is where that difference stays:
//!
//! * **System labels are the mailboxes** — `INBOX`, `SENT`, `DRAFT`,
//!   `TRASH`, `SPAM` surface as Inbox/Sent/Drafts/Trash/Junk with their
//!   roles; user labels surface as plain folders. **Archive is the
//!   messages no system label claims**: moving there removes the source
//!   label and adds nothing, which is exactly what "archive" means to a
//!   Gmail account (ADR 0018 Q4), and listing it is a search over
//!   everything outside the system labels.
//! * **Flags are labels, one of them inverted** — `\Seen` is the *absence*
//!   of `UNREAD`, `\Flagged` is `STARRED`. `\Deleted` maps to the trash:
//!   the seam's mark-then-expunge becomes trash-then-permanent-delete, and
//!   only ever for the ids the caller named.
//! * **Identity is the Gmail message id verbatim**; the uid is a synthetic
//!   enumeration position, exactly like the JMAP adapter, made safe by the
//!   identity-first upsert (#544).
//!
//! Like the JMAP adapter's first slice: no CondStore claim (resyncs
//! re-enumerate, refreshed in place per #564), whole-message body fetches
//! only (`format=raw`), and delta sync over `history.list` is ADR 0018
//! Q3's later seam. `find_by_message_id` *is* implemented — Gmail's
//! search speaks `rfc822msgid:` natively.
//!
//! Credentials are OAuth bearers from the account's `TokenSource`
//! (ADR 0006): the user's own client or a broker until #195 clears CASA,
//! which is why the preset row keeps IMAP first (a data flip later).
//!
//! [`MailBackend`]: postio_imap::backend::MailBackend

pub mod backend;
mod connection;
mod convert;

pub use backend::GmailBackend;
