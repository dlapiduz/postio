use postio_account::backend::BackendError;

use io_jmap::client::JmapClientStdError;

/// `UidValidityChanged` never comes out of here: JMAP ids have no generations (ADR 0018 Q2).
pub(crate) fn backend_error(context: &str, error: JmapClientStdError) -> BackendError {
    match error {
        JmapClientStdError::Io(source) => BackendError::Io {
            context: context.to_owned(),
            reason: source.to_string(),
        },
        other => {
            let reason = other.to_string();
            // A session fetch failing with an auth status is a credential problem, not a retry.
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
