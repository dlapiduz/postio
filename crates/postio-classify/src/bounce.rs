//! A pure function over a message's `Content-Type` and delivery-status text: it names the failed
//! address only for a permanent (`5.x.x`) failure in a `multipart/report;
//! report-type=delivery-status`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounce {
    pub address: String,
    pub status: String,
}

/// `sender` is unused: a report is a report whoever sends it.
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
            "final-recipient" if address.is_none() => {
                let who = value.rsplit_once(';').map_or(value, |(_, who)| who).trim();
                if is_address(who) {
                    address = Some(who.to_ascii_lowercase());
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
