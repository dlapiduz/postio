//! The privacy pane's words, shared so both apps make the same claim in the same words (#151).

use postio_model::egress::EgressEvent;

pub const ALLOWED: &str = "Remote images allowed";
pub const LISTS_LEFT: &str = "Mailing lists left";
pub const READ_RECEIPTS: &str = "Read receipts";
pub const CONNECTIONS: &str = "Recent connections";

pub const NO_ALLOWED: &str = "No senders are always allowed to load remote images.";
pub const NO_LISTS_LEFT: &str = "No mailing lists have been left yet.";
pub const NO_CONNECTIONS: &str = "Nothing has connected out yet this session.";

pub const CONNECTION_WHEN: &str = "%d %b %H:%M";
pub const LEFT_WHEN: &str = "%Y-%m-%d";

/// An audit surface, not an archive: the newest screenful answers "what has this been talking to".
pub const CONNECTION_ROWS: u32 = 50;

/// A count, not a switch: Postio never sends a receipt automatically.
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
