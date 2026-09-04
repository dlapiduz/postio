//! RFC 5321 conformance, one test per row of `docs/rfc-compliance.md`.
//!
//! The companion to `postio-model`'s `rfc5322.rs`: that file is what keeps the
//! message-format verdicts honest, and this one does the same for what goes on
//! the wire. A row in that document that says "compliant" without a test here
//! is a wish.
//!
//! `io-smtp` is sans-I/O, so every exchange below runs against a recorded
//! transcript with no socket involved — the same `ScriptedConnector`
//! `smtp_session.rs` uses. Nothing here touches the network.
//!
//! Response-code classification (2xx/4xx/5xx, per-step) is covered
//! one-case-per-rejection in `smtp_session.rs` and is not repeated here; this
//! file covers the parts that pass through `DATA` and the envelope.

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

/// A draft to Grace with `body`, and nothing else remarkable.
fn draft_with(body: &str) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.to = vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.net")];
    draft.subject = "Tuesday walkthrough notes".to_owned();
    draft.body.text = Some(body.to_owned());
    draft
}

/// Everything written to the wire, as text.
fn wire(connector: &ScriptedConnector) -> String {
    String::from_utf8_lossy(&connector.log().written).into_owned()
}

/// The bytes between the `DATA` command's reply and the terminator.
///
/// Located by the terminator rather than by an offset: what this file is
/// checking *is* where the payload ends, so an assertion that assumed a
/// length would be assuming the answer.
fn payload(written: &str) -> &str {
    let start = written.find("DATA\r\n").expect("a DATA command") + "DATA\r\n".len();
    let rest = &written[start..];
    let end = rest.find("\r\n.\r\n").expect("the DATA terminator");
    &rest[..end + 2]
}

// ---------------------------------------------------------------------------
// §4.1.1.4 — the DATA terminator, and the CRLF the generator does not write
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_body_whose_last_line_has_no_crlf_is_still_terminated_correctly() {
    // `postio-model` generates a message whose final body line has no CRLF —
    // asserted below rather than assumed, because the whole point of this
    // test is what happens to that message. RFC 5321 §4.1.1.4 makes the
    // terminator `CRLF "." CRLF`, so a sender that wrote the payload and then
    // `.CRLF` would produce `...text.` on one line: the server would never
    // see a terminator, and the message would hang or absorb whatever came
    // next.
    let built = postio_model::outgoing::build(&draft_with("Looking now."), &ada(), &[], None);
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

// ---------------------------------------------------------------------------
// §4.5.2 — transparency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_body_line_that_is_a_bare_dot_is_stuffed_before_it_reaches_the_wire() {
    // `postio-model` will happily produce this: a text body containing
    // `\r\n.\r\n` comes out as a line that is one full stop, under a 7bit
    // transfer encoding that does not disguise it. Unstuffed, that line *is*
    // the terminator, and the message is delivered truncated at the dot with
    // no error anywhere — the recipient simply gets less than was written.
    let built =
        postio_model::outgoing::build(&draft_with("before\r\n.\r\nafter"), &ada(), &[], None);
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

// ---------------------------------------------------------------------------
// §3.3 / §4.1.1.2–3 — the envelope is not the header (and Bcc is why)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bcc_recipient_is_an_envelope_address_and_never_a_header() {
    // `PRODUCT.md` §21's privacy claim, end to end. `outgoing::build` omits
    // the `Bcc` header while `Draft::all_recipients` — which is what the send
    // path hands to the envelope — includes the address. Both halves have to
    // hold: a `Bcc` header would disclose the address to every other
    // recipient, and an envelope without it would silently not deliver.
    let mut draft = draft_with("Looking now.");
    draft.cc = vec![EmailAddress::new(None::<String>, "cc@example.net")];
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.org")];
    let built = postio_model::outgoing::build(&draft, &ada(), &[], None);

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
    // The control: the same message does carry the addresses that are meant
    // to be visible, so the assertions above are not passing on a payload
    // that happens to contain no addresses at all.
    assert!(
        payload.contains("grace@example.net") && payload.contains("cc@example.net"),
        "the To and Cc addresses are missing from the message too, so the \
         two assertions above prove nothing"
    );
}

// ---------------------------------------------------------------------------
// §4.5.3.1.6 — line length
// ---------------------------------------------------------------------------

#[test]
fn no_generated_line_comes_near_the_thousand_octet_limit() {
    // §4.5.3.1.6 sets 1000 octets including CRLF for a text line, and a
    // server may refuse a longer one. Nothing in Postio truncates or folds
    // the body itself — this holds because `mail-builder` picks a transfer
    // encoding that bounds the line, quoted-printable for the long-line case
    // here, and 7bit only when the text already fits.
    for (what, body) in [
        ("one very long line", "x".repeat(1200)),
        ("a long unbroken word", "y".repeat(4000)),
        ("many short lines", "short\r\n".repeat(400)),
    ] {
        let built = postio_model::outgoing::build(&draft_with(&body), &ada(), &[], None);
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

// ---------------------------------------------------------------------------
// §2.2.1 / RFC 6152 / RFC 6531 — extensions Postio neither announces nor needs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_is_announced_on_mail_from_or_rcpt_to() {
    // Postio reads no EHLO capability list and passes no ESMTP parameters, so
    // it cannot rely on an extension the server did not advertise. That is
    // the whole compliance argument for SIZE, 8BITMIME and SMTPUTF8, and it
    // is only true while the commands stay bare — this is what says so.
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
    // RFC 6152: 8-bit content requires the server to have advertised
    // 8BITMIME, and Postio never looks. It gets away with that because
    // `mail-builder` encodes anything that would need 8 bits — base64 here —
    // so the octets on the wire stay under 128. If that ever stopped being
    // true, Postio would be sending 8-bit data to servers that never agreed
    // to receive it, and this is the assertion that would notice.
    //
    // **The message only.** The addresses here are ASCII on purpose, because
    // the envelope is a separate question with a separate answer: a
    // non-ASCII address *does* put 8-bit octets on the wire today, which is
    // #922 and is asserted above. Widening this test to cover both would
    // make it fail for the known gap and stop guarding the body.
    let mut draft = draft_with("Grüße, Grace — φ ≈ 1.618");
    draft.subject = "Grüße".to_owned();
    let built = postio_model::outgoing::build(&draft, &ada(), &[], None);

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

// ---------------------------------------------------------------------------
// §4.2.1 — multiline replies, and RFC 3463 enhanced status codes
// ---------------------------------------------------------------------------

/// The happy-path script with one step answering `reply` instead.
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
    // §4.2.1: a reply is one or more lines, all carrying the same code, with
    // `-` after the code on every line but the last.
    //
    // Every line reaches a person (#921). The actionable half of a bounce
    // from a large provider is routinely on the second and third lines --
    // the typo hint, the help URL -- and a reply that says only "the address
    // does not exist" gives somebody nothing to do about it.
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

    // The half #921 is about.
    assert!(
        reason.contains("Check for typos in the recipient"),
        "the second line of the reply is where the advice was: {reason}"
    );
    assert!(
        reason.contains("https://example.invalid/help/nosuchuser"),
        "and the third is where the explanation was: {reason}"
    );

    // One message, not three. The code is quotable once; three copies of it
    // read as three failures.
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

    // Nothing here may reach a log line as more than one line. `SmtpError`
    // is formatted into `tracing` and into the Attention row, and a reason
    // carrying a CRLF would forge a log entry.
    assert!(
        !reason.contains('\r') && !reason.contains('\n'),
        "a line break survived into the reason: {reason:?}"
    );
}

#[tokio::test]
async fn an_enhanced_status_code_reaches_the_reason_though_the_first_digit_decides() {
    // RFC 3463 codes are supplementary: §4.2.1 makes the *first digit* the
    // retry decision, and that is what `classify` uses. The enhanced code is
    // still the most specific thing the server said, so it has to survive
    // into the reason a log or a person sees — otherwise the detail is
    // gathered and thrown away.
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

    // Same enhanced class, permanent basic code: the first digit is what
    // separates them, which is exactly the rule this is checking.
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

/// A script whose EHLO advertises `SMTPUTF8`.
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

/// RFC 6531 §3.1 / RFC 5891: a non-ASCII **domain** needs no extension.
///
/// IDNA turns it into ASCII, and every server accepts ASCII — so this is a
/// conversion rather than a negotiation, and it happens whatever the server
/// advertised. The script here is the plain one, with no `SMTPUTF8` in its
/// EHLO, precisely to show the send does not depend on it.
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

/// RFC 6531: a non-ASCII **local part** is only legal once the server has
/// advertised `SMTPUTF8`, so without it Postio refuses rather than guessing.
///
/// Refusing is the whole point. A server that never advertised the extension
/// is entitled to reject the command — and some accept it and mangle the
/// address instead, which reaches the user as the recipient ignoring their
/// mail. Neither is something to find out about after the fact.
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

    // And nothing went out. A refusal that had already sent `RCPT TO` would
    // be a refusal after the fact.
    let commands = connector.log().commands();
    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("RCPT TO")),
        "the address reached the wire before being refused: {commands:?}"
    );
}

/// And when the server *does* advertise it, the address goes out as typed —
/// with the `SMTPUTF8` parameter RFC 6531 §3.4 requires on `MAIL FROM`.
///
/// The parameter is not decoration: it is what tells the server the
/// transaction contains UTF-8, and an implementation that sent the 8-bit
/// address without it would be relying on an extension it never asked for,
/// which is the same fault as not reading the capability at all.
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
