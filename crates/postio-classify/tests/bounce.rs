//! Reading a delivery failure (specs/011-focus-contacts S10): a
//! `multipart/report; report-type=delivery-status` message with a 5.x.x
//! status names the address that failed; nothing else does. Invented mail on
//! reserved domains.

use postio_classify::bounce::{Bounce, detect};

const DSN: &str = "Reporting-MTA: dns; mx4.example.net\r\nArrival-Date: Fri, 6 Mar 2026 07:20:00 +0000\r\n\r\nFinal-Recipient: rfc822; a.moreno@example.net\r\nAction: failed\r\nStatus: 5.1.1\r\nDiagnostic-Code: smtp; 550 5.1.1 User unknown\r\n";

#[test]
fn a_permanent_delivery_failure_names_the_address() {
    let found = detect(
        Some("multipart/report; report-type=delivery-status; boundary=\"b\""),
        Some("MAILER-DAEMON@mx4.example.net"),
        DSN,
    );
    assert_eq!(
        found,
        Some(Bounce {
            address: "a.moreno@example.net".to_owned(),
            status: "5.1.1".to_owned(),
        })
    );
}

#[test]
fn a_temporary_failure_is_not_a_bounce() {
    let delayed = DSN.replace("5.1.1", "4.4.1").replace("failed", "delayed");
    assert_eq!(
        detect(
            Some("multipart/report; report-type=delivery-status"),
            None,
            &delayed
        ),
        None
    );
}

#[test]
fn a_message_that_is_not_a_delivery_report_is_not_read() {
    assert_eq!(
        detect(Some("text/plain"), Some("ada@example.org"), DSN),
        None
    );
    assert_eq!(detect(None, None, DSN), None);
    assert_eq!(
        detect(
            Some("multipart/report; report-type=disposition-notification"),
            None,
            DSN
        ),
        None,
        "a read receipt is not a bounce"
    );
}

#[test]
fn a_report_with_no_recipient_or_a_malformed_one_says_nothing() {
    let none = "Reporting-MTA: dns; mx4.example.net\r\nStatus: 5.1.1\r\n";
    let kind = Some("multipart/report; report-type=delivery-status");
    assert_eq!(detect(kind, None, none), None);
    let junk = "Final-Recipient: rfc822; not an address\r\nStatus: 5.1.1\r\n";
    assert_eq!(detect(kind, None, junk), None);
}

#[test]
fn the_address_is_lowercased_and_an_original_recipient_line_is_not_mistaken_for_it() {
    let body = "Original-Recipient: rfc822; list@example.net\r\nFinal-Recipient: rfc822; A.Moreno@Example.NET\r\nStatus: 5.2.2\r\n";
    let found = detect(
        Some("multipart/report; report-type=delivery-status"),
        None,
        body,
    )
    .unwrap();
    assert_eq!(found.address, "a.moreno@example.net");
    assert_eq!(found.status, "5.2.2");
}
