//! Mailbox selection, cached per connection.
//!
//! A pooled connection is reused across many small fetches — a
//! ten-thousand-message backfill calls into the same session chunk after
//! chunk, per `UidSet::chunks` — and reselecting the same mailbox before
//! every one of them would double the round trips for no reason. So the
//! session remembers what it last selected, and in what mode, and only
//! issues `SELECT` again when the mailbox changes or a caller needs
//! `CONDSTORE` and the cached selection does not have it.
//!
//! `CONDSTORE`, once selected on a mailbox, stays active for the life of that
//! selection (RFC 7162 §3.1.4), which is what makes the "reuse if it already
//! has condstore" half of the cache check correct.

use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::select::ImapMailboxSelectOptions;
use io_imap::types::command::SelectParameter;
use postio_model::UidValidity;

use crate::backend::{BackendError, Capability};

use super::mailboxes::mailbox_argument;
use super::{ImapSession, map_client_error};
use crate::backend::BackendResult;

/// What is currently selected on a session, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SelectedMailbox {
    path: String,
    condstore: bool,
    uid_validity: UidValidity,
}

impl ImapSession {
    /// Selects `path` unless it is already selected in a mode that covers
    /// what the caller needs, and returns its `UIDVALIDITY`.
    ///
    /// `want_condstore` asks for `SELECT (CONDSTORE)`, which RFC 7162 §3.3.1
    /// requires before a `FETCH … (CHANGEDSINCE n)` on this mailbox. Asking
    /// for it against a server that never advertised `CONDSTORE` is
    /// [`BackendError::Unsupported`], not a silent fallback.
    pub(crate) async fn ensure_selected(
        &mut self,
        path: &str,
        want_condstore: bool,
    ) -> BackendResult<UidValidity> {
        if let Some(selected) = &self.selected
            && selected.path == path
            && (selected.condstore || !want_condstore)
        {
            return Ok(selected.uid_validity);
        }

        if want_condstore {
            self.capabilities().require(Capability::CondStore)?;
        }

        let mailbox = mailbox_argument(path)?;
        let parameters = if want_condstore {
            vec![SelectParameter::CondStore]
        } else {
            Vec::new()
        };

        let data = self
            .select(mailbox, ImapMailboxSelectOptions { parameters })
            .await
            .map_err(|error| map_client_error("SELECT", self.account(), error))?;

        let uid_validity = data
            .uid_validity
            .map(|value| UidValidity::new(value.get()))
            .ok_or_else(|| BackendError::Protocol {
                reason: format!("{path} SELECT carried no UIDVALIDITY"),
            })?;

        self.selected = Some(SelectedMailbox {
            path: path.to_owned(),
            condstore: want_condstore,
            uid_validity,
        });

        Ok(uid_validity)
    }
}
