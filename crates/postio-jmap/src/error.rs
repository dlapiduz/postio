//! Mapping `io-jmap` failures onto the seam's [`BackendError`].

use postio_imap::backend::BackendError;

use io_jmap::client::JmapClientStdError;

/// Coarse but honest: the seam's error taxonomy decides retry behaviour,
/// so what matters is transient-vs-permanent and auth-vs-everything-else.
/// `UidValidityChanged` can never come out of here — JMAP ids have no
/// generations to invalidate, which is the point of ADR 0018 Q2.
pub(crate) fn backend_error(context: &str, error: JmapClientStdError) -> BackendError {
    match error {
        JmapClientStdError::Io(source) => BackendError::Io {
            context: context.to_owned(),
            reason: source.to_string(),
        },
        other => {
            let reason = other.to_string();
            // The session fetch is the authenticated doorstep: a failure
            // there that mentions the HTTP auth statuses is a credential
            // problem the user has to act on, not a retry.
            if reason.contains("401") || reason.contains("403") {
                BackendError::Auth {
                    account: String::new(),
                    reason,
                }
            } else {
                BackendError::Protocol { reason }
            }
        }
    }
}
