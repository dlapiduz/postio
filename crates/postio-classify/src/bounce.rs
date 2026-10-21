//! Reading a delivery failure (specs/011-focus-contacts S10).
//!
//! A pure function over the message's `Content-Type` and the text of its
//! delivery-status part. It names the address that failed, and only for a
//! permanent failure (a `5.x.x` status) in a
//! `multipart/report; report-type=delivery-status` message; a delay, a read
//! receipt and ordinary mail say nothing. No I/O, no network.

/// A permanent delivery failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounce {
    /// The address that could not be delivered to, lowercased.
    pub address: String,
    /// The status that said so, `5.1.1`.
    pub status: String,
}

/// The failure `body` reports, if `content_type` says it is a delivery report.
///
/// `sender` is who the report came from; it is not needed to decide, and is
/// kept so a caller that has it can say so, but a report is a report whoever
/// sends it.
pub fn detect(content_type: Option<&str>, sender: Option<&str>, body: &str) -> Option<Bounce> {
    let _ = sender;
    let kind = content_type?.to_ascii_lowercase();
    if !kind.starts_with("multipart/report") || !kind.contains("report-type=delivery-status") {
        return None;
    }
    let mut status = None;
    let mut address = None;
    for line in body.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "status" => {
                if status.is_none() {
                    status = Some(value.split_whitespace().next().unwrap_or("").to_owned());
                }
            }
            "final-recipient" => {
                if address.is_none() {
                    // `rfc822; a.moreno@example.net`
                    let who = value.rsplit_once(';').map_or(value, |(_, who)| who).trim();
                    if is_address(who) {
                        address = Some(who.to_ascii_lowercase());
                    }
                }
            }
            _ => {}
        }
    }
    let status = status?;
    if !status.starts_with("5.") {
        return None;
    }
    Some(Bounce {
        address: address?,
        status,
    })
}

fn is_address(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !text.contains(char::is_whitespace)
        && !domain.ends_with('.')
}
