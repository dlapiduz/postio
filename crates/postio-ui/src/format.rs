//! How Postio writes numbers a person reads, shared so two surfaces never format one figure two
//! ways (#411).

use postio_core::event::MailFootprint;

/// The one place account names are joined, so the list banner and the selection summary
/// spell the same list the same way (#811). Every name, never "and 2 others".
pub fn names(accounts: &[String]) -> String {
    match accounts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Binary units, matching `postio-search`'s `larger:`/`smaller:` parser, so a part shown as `1.0
/// MB` is one `larger:1M` finds.
pub fn human_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    let (value, unit) = if bytes < KIB {
        return format!("{bytes:.0} B");
    } else if bytes < KIB * KIB {
        (bytes / KIB, "KB")
    } else if bytes < KIB * KIB * KIB {
        (bytes / (KIB * KIB), "MB")
    } else {
        (bytes / (KIB * KIB * KIB), "GB")
    };
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// `MailFootprint::complete == false` means every byte figure is a lower bound, so the hedge is
/// part of the number (#411, ADR 0017).
pub fn human_size_bound(bytes: u64, complete: bool) -> String {
    let size = human_size(bytes);
    if complete {
        size
    } else {
        format!("over {size}")
    }
}

/// `890 MB downloaded · attachments would add 11 GB`, or with `attachments_included`
/// (`[sync] attachment_fetch == "eager"`) `890 MB of 12 GB downloaded · attachments included`.
/// `None` for an empty or unmeasured account, since `0 B` reads as a bug. While the header
/// pass runs, totals are lower bounds (`over 11 GB`) and downloaded bytes are exact.
pub fn mail_weight(footprint: &MailFootprint, attachments_included: bool) -> Option<String> {
    if footprint.total_bytes == 0 {
        return None;
    }
    let local = human_size(footprint.local_bytes);
    if attachments_included {
        let total = human_size_bound(footprint.total_bytes, footprint.complete);
        return Some(format!(
            "{local} of {total} downloaded · attachments included"
        ));
    }
    if footprint.attachment_bytes == 0 {
        return Some(format!("{local} downloaded"));
    }
    let payloads = human_size_bound(footprint.attachment_bytes, footprint.complete);
    Some(format!(
        "{local} downloaded · attachments would add {payloads}"
    ))
}

/// Both ends carry information and the middle (often a relay's random token) does not (#1008).
/// A string at or under `width` is unchanged.
pub fn middle_truncate(text: &str, width: usize) -> String {
    // By characters, not bytes: slicing a UTF-8 string at a byte offset can panic.
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= width {
        return text.to_owned();
    }
    if width <= 1 {
        return "…".to_owned();
    }
    let keep = width - 1;
    // The tail is the more identifying half, so it gets the odd character.
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = characters[..head].iter().collect();
    out.push('…');
    out.extend(&characters[characters.len() - tail..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_the_way_the_canvas_writes_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(11 * 1024), "11 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn an_incomplete_count_says_so_rather_than_claiming_a_total() {
        assert_eq!(human_size_bound(1_503_238_553, false), "over 1.4 GB");
        assert_eq!(human_size_bound(1_503_238_553, true), "1.4 GB");
    }

    #[test]
    fn an_account_row_says_what_its_mail_weighs_and_what_attachments_would_add() {
        let footprint = MailFootprint {
            total_bytes: 12_884_901_888,
            attachment_bytes: 11_811_160_064,
            local_bytes: 933_232_640,
            complete: true,
        };

        assert_eq!(
            mail_weight(&footprint, false).as_deref(),
            Some("890 MB downloaded \u{b7} attachments would add 11 GB"),
            "a policy that is not fetching payloads owes the cost of switching"
        );
        assert_eq!(
            mail_weight(&footprint, true).as_deref(),
            Some("890 MB of 12 GB downloaded \u{b7} attachments included"),
            "a policy that is fetching them has a total worth showing against"
        );
    }

    #[test]
    fn a_row_whose_totals_are_still_being_counted_hedges_every_one_of_them() {
        let counting = MailFootprint {
            total_bytes: 12_884_901_888,
            attachment_bytes: 11_811_160_064,
            local_bytes: 933_232_640,
            complete: false,
        };

        assert_eq!(
            mail_weight(&counting, false).as_deref(),
            Some("890 MB downloaded \u{b7} attachments would add over 11 GB"),
        );
        assert_eq!(
            mail_weight(&counting, true).as_deref(),
            Some("890 MB of over 12 GB downloaded \u{b7} attachments included"),
        );
    }

    #[test]
    fn a_row_with_nothing_to_weigh_makes_no_claim_at_all() {
        assert_eq!(mail_weight(&MailFootprint::default(), false), None);
        assert_eq!(mail_weight(&MailFootprint::default(), true), None);

        let no_payloads = MailFootprint {
            total_bytes: 933_232_640,
            attachment_bytes: 0,
            local_bytes: 933_232_640,
            complete: true,
        };
        assert_eq!(
            mail_weight(&no_payloads, false).as_deref(),
            Some("890 MB downloaded")
        );
    }

    #[test]
    fn a_short_address_is_left_alone() {
        assert_eq!(middle_truncate("ada@example.com", 40), "ada@example.com");
        assert_eq!(middle_truncate("ada@example.com", 15), "ada@example.com");
    }

    #[test]
    fn a_relay_address_keeps_both_ends() {
        let relay = "transaction_at_shop_12345@privaterelay.appleid.example";
        let short = middle_truncate(relay, 30);
        assert_eq!(short.chars().count(), 30, "{short}");
        assert!(short.starts_with("transaction"), "{short}");
        assert!(short.ends_with("appleid.example"), "{short}");
        assert!(short.contains('…'), "{short}");
    }

    #[test]
    fn the_tail_gets_the_odd_character() {
        let short = middle_truncate("abcdefghij", 6);
        assert_eq!(short, "ab…hij");
    }

    #[test]
    fn a_width_with_no_room_is_just_the_marker() {
        assert_eq!(middle_truncate("abcdefghij", 1), "…");
        assert_eq!(middle_truncate("abcdefghij", 0), "…");
    }

    #[test]
    fn a_multibyte_address_does_not_split_a_character() {
        let short = middle_truncate("ünïcödé_sender@exämple.test", 12);
        assert_eq!(short.chars().count(), 12, "{short}");
    }
}
