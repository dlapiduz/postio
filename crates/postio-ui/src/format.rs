//! How Postio writes numbers a person reads.
//!
//! One helper per kind of number, shared by every surface that shows it.
//! Two surfaces formatting bytes their own way is not a cosmetic problem:
//! the status line saying `1.4 GB` while the settings panel says `1,400 MB`
//! reads as two different measurements of two different things (#411).

use postio_core::event::MailFootprint;

/// "Personal", "Personal and Work", "Personal, Work and Archive".
///
/// The one place Postio joins a list of account names, because the surfaces
/// that name absent accounts have to name them the same way: the list pane's
/// banner says which accounts a unified view could not reach, and the
/// selection summary says which ones a whole-view selection therefore left
/// out. Two spellings of the same list read as two different lists (#811,
/// ADR 0005 Q10).
///
/// Every name, never "and 2 others": naming one of three absent accounts is
/// its own omission, and the list is bounded by how many accounts a person
/// configures.
pub fn names(accounts: &[String]) -> String {
    match accounts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A size, the way the canvas writes it: `11 KB`, `1.1 MB`.
///
/// Binary units, matching `postio-search`'s `larger:`/`smaller:` parser — a
/// part the reader calls `1.0 MB` has to be one `larger:1M` finds, or the two
/// halves of the application disagree about what a megabyte is.
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
    // One decimal below ten, none above: `1.1 MB` and `340 KB`, never
    // `1.1 KB` sitting in a column beside `340.2 KB`.
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// A size that is still being counted: `over 1.4 GB`.
///
/// `MailFootprint::complete == false` means every byte figure is a lower
/// bound, and a surface has to say so. A total that silently climbs every few
/// seconds reads as a bug, and one that is simply wrong is worse than one
/// that admits it is not finished — so the hedge is part of the number rather
/// than a footnote somewhere else on the screen (#411, ADR 0017).
pub fn human_size_bound(bytes: u64, complete: bool) -> String {
    let size = human_size(bytes);
    if complete {
        size
    } else {
        format!("over {size}")
    }
}

/// What one account's mail weighs, for the settings panel's account row.
///
/// `890 MB downloaded · attachments would add 11 GB` when payloads are not
/// being fetched, and `890 MB of 12 GB downloaded · attachments included`
/// when they are. The two shapes differ because the questions do: a policy
/// that is not pulling payloads has no meaningful total to show progress
/// against, and one that is has no cost left to quote (#411).
///
/// `attachments_included` is `[sync] attachment_fetch == "eager"`. That
/// setting is global and this figure is per account, which is why the number
/// lives on the row rather than beside the setting: summing across accounts
/// in a reader's head is fine, and inventing a global total to sit beside a
/// global setting is not, because there is no row to read one off.
///
/// `None` — no line at all — where a size would be a claim rather than a
/// fact:
///
/// * **nothing measured, or an empty account** — `0 B` reads as a bug, not as
///   "no mail". The same rule the status line follows.
///
/// An account with mail but **no payloads** keeps the line and drops the cost
/// clause: what attachments would add is nothing, and a line saying so makes
/// a reader look for the catch.
///
/// While the header pass is still running every *total* is a lower bound and
/// is written `over 11 GB`. What is already downloaded is known exactly and
/// carries no hedge — hedging it too would say the local figure might grow
/// for a different reason than it will.
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

/// An address or path, shortened from the middle: `transaction_at_…y.com`.
///
/// Both ends carry information and the middle does not: the local part says
/// who, the domain says where, and the random string between them says only
/// that a relay generated it. Apple's private-relay addresses are the case
/// this exists for — 70 characters that turned the reader's remote-image
/// notice into three lines of chrome above somebody's mail (#1008).
///
/// `…` rather than `...`: this goes in a menu, where one glyph is the
/// difference between an elision and a filename.
///
/// A string already at or under `width` is returned unchanged, so a short
/// address never grows a decoration it does not need.
pub fn middle_truncate(text: &str, width: usize) -> String {
    // By characters, not bytes: an address can carry non-ASCII, and slicing
    // a UTF-8 string at a byte offset is a panic waiting for the first one.
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= width {
        return text.to_owned();
    }
    // Under three there is no room for two ends and a marker, and returning
    // something misleadingly short would be worse than an ellipsis alone.
    if width <= 1 {
        return "…".to_owned();
    }
    let keep = width - 1;
    // The tail is the more identifying half — a domain distinguishes two
    // relay addresses that share a prefix — so it gets the odd character.
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
        // The state the issue calls the one most likely to be got wrong: the
        // header pass has not finished, so this number will grow.
        assert_eq!(human_size_bound(1_503_238_553, false), "over 1.4 GB");
        assert_eq!(human_size_bound(1_503_238_553, true), "1.4 GB");
    }

    /// Issue #411's account row, whose whole point is that a policy is an
    /// abstraction and a number is a decision.
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
        // `0 B` reads as a fault rather than as "no mail here" -- the same
        // rule the status line follows, and for the same reason.
        assert_eq!(mail_weight(&MailFootprint::default(), false), None);
        assert_eq!(mail_weight(&MailFootprint::default(), true), None);

        // An account with mail but no attachments is not owed a sentence
        // about what attachments would cost: the answer is nothing, and
        // "would add 0 B" is a line that makes a reader look for the catch.
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
        // The case the notice bar exists to survive: 70 characters, of which
        // the middle is a machine-generated token that identifies nothing.
        let relay = "transaction_at_shop_12345@privaterelay.appleid.example";
        let short = middle_truncate(relay, 30);
        assert_eq!(short.chars().count(), 30, "{short}");
        assert!(short.starts_with("transaction"), "{short}");
        assert!(short.ends_with("appleid.example"), "{short}");
        assert!(short.contains('…'), "{short}");
    }

    #[test]
    fn the_tail_gets_the_odd_character() {
        // A domain distinguishes two relay addresses that share a prefix, so
        // when the two halves cannot be equal the far end is the one to keep.
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
        // Slicing at a byte offset would panic here rather than truncate.
        let short = middle_truncate("ünïcödé_sender@exämple.test", 12);
        assert_eq!(short.chars().count(), 12, "{short}");
    }
}
