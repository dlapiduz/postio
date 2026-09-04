//! Against the real Gmail API — `#[ignore]`, never in the default suite.
//!
//! ```sh
//! POSTIO_GMAIL_TOKEN=<an OAuth access token with the mail scope> \
//!   cargo test -p postio-gmail --test gmail_live -- --ignored
//! ```

use postio_account::backend::{MailBackend, MailboxFilter};
use postio_gmail::GmailBackend;
use postio_model::MailboxRole;

#[tokio::test]
#[ignore = "dials the real Gmail API; needs POSTIO_GMAIL_TOKEN"]
async fn the_profile_resolves_and_the_inbox_is_listed() {
    let Ok(token) = std::env::var("POSTIO_GMAIL_TOKEN") else {
        eprintln!("skipping: POSTIO_GMAIL_TOKEN not set");
        return;
    };
    let backend = GmailBackend::new(&token);
    backend.connect().await.expect("connect");
    let mailboxes = backend
        .list_mailboxes(&MailboxFilter::default())
        .await
        .expect("list");
    assert!(mailboxes.iter().any(|m| m.role == MailboxRole::Inbox));
}
