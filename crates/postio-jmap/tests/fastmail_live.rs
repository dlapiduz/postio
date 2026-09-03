//! Against a real Fastmail account — `#[ignore]`, never in the default
//! suite (CLAUDE.md: no test in the default suite touches the network).
//!
//! ```sh
//! POSTIO_JMAP_URL=https://api.fastmail.com/jmap/session/ \
//! POSTIO_JMAP_TOKEN=<an api token> \
//!   cargo test -p postio-jmap --test fastmail_live -- --ignored
//! ```

use postio_account::backend::{MailBackend, MailboxFilter};
use postio_jmap::JmapBackend;
use postio_model::MailboxRole;

fn live() -> Option<JmapBackend> {
    let url = std::env::var("POSTIO_JMAP_URL").ok()?.parse().ok()?;
    let token = std::env::var("POSTIO_JMAP_TOKEN").ok()?;
    Some(JmapBackend::new(url, &token))
}

#[tokio::test]
#[ignore = "dials a real JMAP server; needs POSTIO_JMAP_URL and POSTIO_JMAP_TOKEN"]
async fn the_session_resolves_and_the_inbox_is_listed() {
    let Some(backend) = live() else {
        eprintln!("skipping: POSTIO_JMAP_URL / POSTIO_JMAP_TOKEN not set");
        return;
    };

    let capabilities = backend.connect().await.expect("connect");
    assert!(
        capabilities
            .names()
            .iter()
            .any(|name| name.contains("jmap:mail")),
        "{:?}",
        capabilities.names()
    );

    let mailboxes = backend
        .list_mailboxes(&MailboxFilter::default())
        .await
        .expect("list mailboxes");
    assert!(
        mailboxes
            .iter()
            .any(|mailbox| mailbox.role == MailboxRole::Inbox),
        "a live account has an inbox: {:?}",
        mailboxes.iter().map(|m| &m.path).collect::<Vec<_>>()
    );
}
