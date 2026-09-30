//! The privacy pane's words: what it calls each list, what it says when a
//! list is empty, and how it sets out a connection.
//!
//! The pane is the privacy claim made auditable (#151), so both apps must
//! make the same claim in the same words. It lived in `postio-gtk::settings`
//! until spec 005 gave the terminal the same pane.

use postio_model::egress::EgressEvent;

/// The senders whose remote images load without asking (#871).
pub const ALLOWED: &str = "Remote images allowed";
/// The mailing lists left through one-click unsubscribe (#971).
pub const LISTS_LEFT: &str = "Mailing lists left";
/// How many messages asked for a read receipt (#970).
pub const READ_RECEIPTS: &str = "Read receipts";
/// The connections Postio opened (#151).
pub const CONNECTIONS: &str = "Recent connections";

/// [`ALLOWED`], with nobody on it.
pub const NO_ALLOWED: &str = "No senders are always allowed to load remote images.";
/// [`LISTS_LEFT`], with nothing on it.
pub const NO_LISTS_LEFT: &str = "No mailing lists have been left yet.";
/// [`CONNECTIONS`], with nothing on it: on a machine that has never synced,
/// exactly the claim.
pub const NO_CONNECTIONS: &str = "Nothing has connected out yet this session.";

/// How a connection's time is set, in local time.
pub const CONNECTION_WHEN: &str = "%d %b %H:%M";
/// How the day a list was left is set.
pub const LEFT_WHEN: &str = "%Y-%m-%d";

/// How many connections the pane lists. An audit surface, not an archive:
/// the store keeps everything, and the newest screenful answers "what has
/// this thing been talking to".
pub const CONNECTION_ROWS: u32 = 50;

/// The read-receipt count as a sentence. A count, not a switch: Postio never
/// sends a receipt automatically, so there is nothing to turn off.
pub fn read_receipts(count: u64) -> String {
    match count {
        0 => "No messages have requested a read receipt.".to_owned(),
        1 => "1 message has requested a read receipt; none have been sent \
              automatically."
            .to_owned(),
        n => format!(
            "{n} messages have requested a read receipt; none have been \
             sent automatically."
        ),
    }
}

/// What a connection was: who opened it, and to where.
pub fn connection(event: &EgressEvent) -> String {
    format!(
        "{} · {}:{}",
        event.subsystem.as_str(),
        event.host,
        event.port
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_read_receipt_count_says_none_were_sent() {
        assert_eq!(
            read_receipts(0),
            "No messages have requested a read receipt."
        );
        assert!(read_receipts(1).starts_with("1 message has"));
        assert!(read_receipts(3).starts_with("3 messages have"));
        assert!(read_receipts(3).ends_with("none have been sent automatically."));
    }

    #[test]
    fn a_connection_names_who_opened_it_and_where_it_went() {
        let event = EgressEvent {
            at: chrono::DateTime::UNIX_EPOCH,
            subsystem: postio_model::egress::EgressSubsystem::Imap,
            account: None,
            host: "imap.example.com".into(),
            port: 993,
            outcome: postio_model::egress::EgressOutcome::Connected,
        };
        assert_eq!(connection(&event), "imap · imap.example.com:993");
    }
}
