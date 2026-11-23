//! RFC 5321 conformance, one test per row of `docs/rfc-compliance.md`: a row saying "compliant"
//! without a test here is a wish. Runs against a recorded transcript; response-code classification
//! is in `smtp_session.rs`.

use postio_model::draft::Draft;
use postio_model::ids::{AccountId, IdentityId};
use postio_model::{EmailAddress, Identity, TransportSecurity};
use postio_smtp::cancel::CancelToken;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::{ConnectionSettings, SMTPS_PORT};
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use secrecy::SecretString;

fn settings() -> ConnectionSettings {
    ConnectionSettings::new(
        "smtp.example.com",
        SMTPS_PORT,
        TransportSecurity::Tls,
        "ada@example.com",
    )
}

fn happy_script() -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "235 2.7.0 authentication successful")
        .on("MAIL FROM", "250 2.1.0 sender ok")
        .on("RCPT TO", "250 2.1.5 recipient ok")
        .on("DATA", "354 start mail input")
}

async fn open(script: SmtpScript) -> (SmtpSession, ScriptedConnector) {
    let connector = ScriptedConnector::new(script);
    let session = SmtpSession::open(
        &settings(),
        &SecretString::from("app-specific-password"),
        &connector,
    )
    .await
    .expect("the scripted handshake");
    (session, connector)
}

fn ada() -> Identity {
    let mut identity = Identity::new(
        AccountId::UNASSIGNED,
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    identity.id = IdentityId::new(1);
    identity
}

fn draft_with(body: &str) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.to = vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.net")];
    draft.subject = "Tuesday walkthrough notes".to_owned();
    draft.body.text = Some(body.to_owned());
    draft
}

fn wire(connector: &ScriptedConnector) -> String {
    String::from_utf8_lossy(&connector.log().written).into_owned()
}

fn payload(written: &str) -> &str {
    let start = written.find("DATA\r\n").expect("a DATA command") + "DATA\r\n".len();
    let rest = &written[start..];
    let end = rest.find("\r\n.\r\n").expect("the DATA terminator");
    &rest[..end + 2]
}

#[tokio::test]
async fn the_greeting_names_the_client_by_an_address_literal() {
    // §4.1.3 reserves the address literal for a client with no FQDN; a bare `localhost` makes
    // Stalwart answer `550 5.5.0 Invalid EHLO domain` before `MAIL FROM`.
    let (_session, connector) = open(happy_script()).await;

    let written = wire(&connector);
    assert!(
        written.starts_with("EHLO [127.0.0.1]\r\n"),
        "the session greeted with {:?}",
        written.lines().next()
    );
}

#[tokio::test]
async fn a_body_whose_last_line_has_no_crlf_is_still_terminated_correctly() {
    // `postio-model` generates a body whose last line has no CRLF; a sender writing the payload
    // then `.CRLF` would put `...text.` on one line and the server would never see a terminator.
    let built = postio_model::outgoing::build(&draft_with("Looking now."), &ada(), &[], None, None);
    assert!(
        !built.raw.ends_with(b"\r\n"),
        "this test is about a message that does not end with CRLF; the \
         generator has started producing one that does, so it no longer \
         exercises what it was written for"
    );

    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["grace@example.net".to_owned()],
            &built.raw,
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let written = wire(&connector);
    assert!(
        written.contains("\r\n.\r\n"),
        "the payload was never terminated with CRLF.CRLF"
    );
    let payload = payload(&written);
    assert!(
        payload.ends_with("Looking now.\r\n"),
        "the terminator ran on from the last body line instead of starting \
         its own; the payload ends {:?}",
        &payload[payload.len().saturating_sub(24)..]
    );
}

#[tokio::test]
async fn a_body_line_that_is_a_bare_dot_is_stuffed_before_it_reaches_the_wire() {
    // A body containing `\r\n.\r\n` is, unstuffed, the terminator: the message is delivered
    // truncated with no error anywhere.
    let built =
        postio_model::outgoing::build(&draft_with("before\r\n.\r\nafter"), &ada(), &[], None, None);
    assert!(
        String::from_utf8_lossy(&built.raw)
            .lines()
            .any(|line| line == "."),
        "the generator no longer produces a bare-dot line, so this test does \
         not exercise dot-stuffing any more"
    );

    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["grace@example.net".to_owned()],
            &built.raw,
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let written = wire(&connector);
    let payload = payload(&written);
    assert!(
        payload.contains("\r\n..\r\n"),
        "the bare-dot line was not stuffed to `..`, so the server would read \
         it as the end of the message and deliver everything after it as \
         nothing at all"
    );
    assert!(
        payload.contains("after"),
        "the text after the dot line did not reach the wire"
    );
}

#[tokio::test]
async fn a_bcc_recipient_is_an_envelope_address_and_never_a_header() {
    // Both halves have to hold: a `Bcc` header would disclose the address to every other recipient,
    // and an envelope without it would silently not deliver.
    let mut draft = draft_with("Looking now.");
    draft.cc = vec![EmailAddress::new(None::<String>, "cc@example.net")];
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.org")];
    let built = postio_model::outgoing::build(&draft, &ada(), &[], None, None);

    let recipients: Vec<String> = draft
        .all_recipients()
        .map(|address| address.address.clone())
        .collect();
    assert!(
        recipients.iter().any(|to| to == "quiet@example.org"),
        "the send path builds its envelope from `all_recipients`, and this is \
         the list it would hand over; without the bcc'd address the message \
         is simply not delivered to them"
    );

    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &recipients,
            &built.raw,
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let written = wire(&connector);
    let commands = connector.log().commands();
    let rcpts: Vec<&String> = commands
        .iter()
        .filter(|command| command.starts_with("RCPT TO"))
        .collect();
    assert_eq!(
        rcpts.len(),
        3,
        "one RCPT TO per envelope recipient: {rcpts:?}"
    );
    assert!(
        rcpts.iter().any(|rcpt| rcpt.contains("quiet@example.org")),
        "the bcc'd address never reached the envelope, so the message was \
         not delivered to them at all: {rcpts:?}"
    );

    let payload = payload(&written);
    assert!(
        !payload.to_ascii_lowercase().contains("bcc:"),
        "the message carries a Bcc header, which hands every To and Cc \
         recipient the list of people who were bcc'd"
    );
    assert!(
        !payload.contains("quiet@example.org"),
        "the bcc'd address appears in the bytes every other recipient \
         receives, which is the disclosure Bcc exists to prevent"
    );
    assert!(
        payload.contains("grace@example.net") && payload.contains("cc@example.net"),
        "the To and Cc addresses are missing from the message too, so the \
         two assertions above prove nothing"
    );
}

#[tokio::test]
async fn a_bcc_header_handed_to_the_transport_never_reaches_the_wire() {
    // This holds the transport to the rule on its own, whatever bytes reach it. The header is
    // folded because a filter dropping only its first line would leave the second naming an
    // address.
    let raw = "From: ada@example.com\r\n\
               To: grace@example.net\r\n\
               Bcc: quiet@example.org,\r\n \
               other@example.org\r\n\
               Subject: Draft review\r\n\
               \r\n\
               Looking now.\r\n";
    let recipients = vec![
        "grace@example.net".to_owned(),
        "quiet@example.org".to_owned(),
        "other@example.org".to_owned(),
    ];

    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &recipients,
            raw.as_bytes(),
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let written = wire(&connector);
    let payload = payload(&written);
    assert!(
        !payload.to_ascii_lowercase().contains("bcc:"),
        "the transport sent a Bcc header, which hands every recipient the \
         list of people who were bcc'd: {payload:?}"
    );
    assert!(
        !payload.contains("quiet@example.org") && !payload.contains("other@example.org"),
        "a bcc'd address reached the bytes every recipient receives, the \
         folded line included: {payload:?}"
    );
    assert!(
        payload.contains("To: grace@example.net\r\n")
            && payload.contains("Subject: Draft review\r\n")
            && payload.contains("Looking now."),
        "the rest of the message did not go out as it was given: {payload:?}"
    );
}

#[test]
fn no_generated_line_comes_near_the_thousand_octet_limit() {
    // Nothing truncates or folds the body; this holds because `mail-builder` picks a transfer
    // encoding that bounds the line.
    for (what, body) in [
        ("one very long line", "x".repeat(1200)),
        ("a long unbroken word", "y".repeat(4000)),
        ("many short lines", "short\r\n".repeat(400)),
    ] {
        let built = postio_model::outgoing::build(&draft_with(&body), &ada(), &[], None, None);
        let longest = String::from_utf8_lossy(&built.raw)
            .lines()
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(
            longest + 2 <= 1000,
            "{what} produced a {longest}-octet line, over §4.5.3.1.6's 1000 \
             including CRLF; a server is entitled to refuse it"
        );
    }
}

#[tokio::test]
async fn nothing_is_announced_on_mail_from_or_rcpt_to() {
    // Postio reads no EHLO capability list and passes no ESMTP parameters, so it cannot rely on an
    // extension the server did not advertise; true only while the commands stay bare.
    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["grace@example.net".to_owned()],
            b"From: ada@example.com\r\nTo: grace@example.net\r\n\r\nhi\r\n",
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    for command in connector.log().commands() {
        if command.starts_with("MAIL FROM") || command.starts_with("RCPT TO") {
            let upper = command.to_ascii_uppercase();
            for parameter in ["SIZE=", "BODY=", "SMTPUTF8", "RET=", "ENVID="] {
                assert!(
                    !upper.contains(parameter),
                    "{command:?} carries {parameter}, an ESMTP parameter \
                     nothing here checked the server advertises. A server \
                     that does not implement it is entitled to reject the \
                     command outright."
                );
            }
        }
    }
}

#[tokio::test]
async fn a_non_ascii_message_goes_out_seven_bit_clean() {
    // `mail-builder` encodes anything needing 8 bits, so the octets stay under 128 (RFC 6152). The
    // addresses are ASCII on purpose:
    // a non-ASCII address does put 8-bit octets on the wire (#922), and widening this would stop it
    // guarding the body.
    let mut draft = draft_with("Grüße, Grace — φ ≈ 1.618");
    draft.subject = "Grüße".to_owned();
    let built = postio_model::outgoing::build(&draft, &ada(), &[], None, None);

    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["grace@example.net".to_owned()],
            &built.raw,
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let high: Vec<u8> = connector
        .log()
        .written
        .iter()
        .copied()
        .filter(|byte| *byte >= 0x80)
        .collect();
    assert!(
        high.is_empty(),
        "{} octets above 127 went onto a connection where 8BITMIME was never \
         advertised or requested",
        high.len()
    );
}

fn script_overriding(keyword: &str, reply: &str) -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(keyword, reply)
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "235 2.7.0 authentication successful")
        .on("MAIL FROM", "250 2.1.0 sender ok")
        .on("RCPT TO", "250 2.1.5 recipient ok")
        .on("DATA", "354 start mail input")
}

async fn send_expecting_failure(script: SmtpScript) -> postio_smtp::error::SmtpError {
    let (mut session, _connector) = open(script).await;
    session
        .send_message(
            "ada@example.com",
            &["grace@example.net".to_owned()],
            b"From: ada@example.com\r\nTo: grace@example.net\r\n\r\nhi\r\n",
            &CancelToken::new(),
        )
        .await
        .expect_err("the scripted rejection")
}

#[tokio::test]
async fn a_multiline_rejection_is_classified_by_its_code_however_many_lines_it_has() {
    // Every line reaches a person (#921): the actionable half of a large provider's bounce is on
    // the second and third lines.
    let error = send_expecting_failure(script_overriding(
        "RCPT TO",
        "550-5.1.1 The address you entered does not exist\r\n\
         550-5.1.1 Check for typos in the recipient\r\n\
         550 5.1.1 https://example.invalid/help/nosuchuser",
    ))
    .await;

    let reason = error.to_string();
    assert!(
        reason.contains("does not exist"),
        "not even the first line of a multiline reply survived: {reason}"
    );
    assert!(
        !error.is_transient(),
        "a 5xx is permanent however many lines it arrived on: {reason}"
    );
    assert!(
        reason.contains("550"),
        "the reply code is what a person can quote to their provider: {reason}"
    );

    assert!(
        reason.contains("Check for typos in the recipient"),
        "the second line of the reply is where the advice was: {reason}"
    );
    assert!(
        reason.contains("https://example.invalid/help/nosuchuser"),
        "and the third is where the explanation was: {reason}"
    );

    assert_eq!(
        reason.matches("550").count(),
        1,
        "the code is repeated, so the reason reads as three rejections: {reason}"
    );
    assert_eq!(
        reason.matches("5.1.1").count(),
        1,
        "and so is the enhanced status code, which every line carries: {reason}"
    );

    // `SmtpError` is formatted into `tracing` and the Attention row, so a reason with a CRLF would
    // forge a log entry.
    assert!(
        !reason.contains('\r') && !reason.contains('\n'),
        "a line break survived into the reason: {reason:?}"
    );
}

#[tokio::test]
async fn an_enhanced_status_code_reaches_the_reason_though_the_first_digit_decides() {
    // The first digit is the retry decision; the RFC 3463 enhanced code is the most specific thing
    // said and must survive into the reason.
    let transient = send_expecting_failure(script_overriding(
        "RCPT TO",
        "450 4.2.1 The recipient's mailbox is disabled for now",
    ))
    .await;
    assert!(
        transient.is_transient(),
        "a 4xx must be retryable: {transient}"
    );
    assert!(
        transient.to_string().contains("4.2.1"),
        "the enhanced code was discarded: {transient}"
    );

    let permanent = send_expecting_failure(script_overriding(
        "RCPT TO",
        "550 5.2.1 The recipient's mailbox is disabled",
    ))
    .await;
    assert!(
        !permanent.is_transient(),
        "a 5xx must not be retried: {permanent}"
    );
    assert!(permanent.to_string().contains("5.2.1"));
}

fn utf8_script() -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(
            "EHLO",
            "250-mail.example.com\r\n250-SMTPUTF8\r\n250 AUTH PLAIN",
        )
        .on("AUTH PLAIN", "235 2.7.0 authentication successful")
        .on("MAIL FROM", "250 2.1.0 sender ok")
        .on("RCPT TO", "250 2.1.5 recipient ok")
        .on("DATA", "354 start mail input")
        .on(".", "250 2.0.0 queued")
        .on("QUIT", "221 2.0.0 bye")
}

/// IDNA makes it ASCII, a conversion rather than a negotiation: the plain script has no `SMTPUTF8`,
/// to show the send does not depend on it.
#[tokio::test]
async fn a_non_ascii_domain_is_punycoded_and_needs_no_extension() {
    let (mut session, connector) = open(happy_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["ada@例え.test".to_owned()],
            b"From: ada@example.com\r\nTo: x@example.net\r\n\r\nhi\r\n",
            &CancelToken::new(),
        )
        .await
        .expect("an ASCII-able address is sendable anywhere");

    let rcpt = connector
        .log()
        .commands()
        .into_iter()
        .find(|command| command.starts_with("RCPT TO"))
        .expect("a RCPT TO");
    assert!(
        rcpt.is_ascii(),
        "a domain that IDNA can render in ASCII must not put 8-bit octets on \
         a connection that never negotiated them: {rcpt}"
    );
    assert!(
        rcpt.contains("xn--r8jz45g.test"),
        "the domain has to be the punycode of what was typed, not something \
         else that happens to be ASCII: {rcpt}"
    );
}

/// A server that never advertised the extension may reject the command or accept and mangle the
/// address, which reaches the user as the recipient ignoring their mail. Refusing is the point.
#[tokio::test]
async fn a_non_ascii_local_part_is_refused_when_the_server_never_offered_smtputf8() {
    let (mut session, connector) = open(happy_script()).await;
    let error = session
        .send_message(
            "ada@example.com",
            &["grüße@example.net".to_owned()],
            b"From: ada@example.com\r\nTo: x@example.net\r\n\r\nhi\r\n",
            &CancelToken::new(),
        )
        .await
        .expect_err("the server never offered SMTPUTF8");

    let said = error.to_string();
    assert!(
        said.contains("grüße@example.net"),
        "the refusal has to name the address, or the user cannot act on it: {said}"
    );
    assert!(
        said.to_lowercase().contains("smtputf8"),
        "and name what is missing: {said}"
    );

    let commands = connector.log().commands();
    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("RCPT TO")),
        "the address reached the wire before being refused: {commands:?}"
    );
}

/// The `SMTPUTF8` parameter on `MAIL FROM` is what tells the server the transaction contains UTF-8;
/// sending the 8-bit address without it would rely on an extension never asked for.
#[tokio::test]
async fn a_non_ascii_local_part_goes_out_when_the_server_advertised_smtputf8() {
    let (mut session, connector) = open(utf8_script()).await;
    session
        .send_message(
            "ada@example.com",
            &["grüße@example.net".to_owned()],
            b"From: ada@example.com\r\nTo: x@example.net\r\n\r\nhi\r\n",
            &CancelToken::new(),
        )
        .await
        .expect("the server advertised SMTPUTF8");

    let commands = connector.log().commands();
    let mail = commands
        .iter()
        .find(|command| command.starts_with("MAIL FROM"))
        .expect("a MAIL FROM");
    assert!(
        mail.contains("SMTPUTF8"),
        "RFC 6531 §3.4 puts the parameter on MAIL FROM; without it the server \
         was never told this transaction carries UTF-8: {mail}"
    );
    let rcpt = commands
        .iter()
        .find(|command| command.starts_with("RCPT TO"))
        .expect("a RCPT TO");
    assert!(
        rcpt.contains("grüße@example.net"),
        "the local part cannot be punycoded -- IDNA is a domain encoding -- so \
         with the extension negotiated it goes out as typed: {rcpt}"
    );
}
