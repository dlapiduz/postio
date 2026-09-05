//! Opening an SMTP session and sending one message: `EHLO`, `AUTH PLAIN`,
//! the mail transaction, and how a rejection at each step is classified.
//!
//! `io-smtp` is sans-I/O, so the whole exchange runs here against a recorded
//! transcript with no socket involved.

use postio_model::TransportSecurity;
use postio_smtp::cancel::CancelToken;
use postio_smtp::error::SmtpError;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::{ConnectionSettings, SMTPS_PORT, SUBMISSION_PORT};
use postio_smtp::transport::{RustlsConnector, ScriptedConnector, SmtpScript};
use secrecy::SecretString;

fn password() -> SecretString {
    SecretString::from("app-specific-password")
}

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

/// The happy-path script with one step replying `reply` instead.
///
/// `SmtpScript::on` matches the first registered rule that fits, so the
/// override is registered before the happy-path rule for the same keyword
/// rather than appended after it.
fn script_overriding(keyword: &str, reply: &str) -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(keyword, reply)
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "235 2.7.0 authentication successful")
        .on("MAIL FROM", "250 2.1.0 sender ok")
        .on("RCPT TO", "250 2.1.5 recipient ok")
        .on("DATA", "354 start mail input")
}

const MESSAGE: &[u8] =
    b"From: Ada Lovelace <ada@example.com>\r\nTo: Grace Hopper <grace@example.com>\r\n\
      Subject: hello\r\n\r\nHello, Grace.\r\n";

async fn open(script: SmtpScript) -> (SmtpSession, ScriptedConnector) {
    let connector = ScriptedConnector::new(script);
    let session = SmtpSession::open(&settings(), &password(), &connector)
        .await
        .expect("the scripted handshake");
    (session, connector)
}

// ---------------------------------------------------------------------------
// The happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_message_is_sent_through_mail_rcpt_and_data() {
    let (mut session, connector) = open(happy_script()).await;

    session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .expect("the scripted send");

    let commands = connector.log().commands();
    assert!(commands.iter().any(|c| c.contains("MAIL FROM")));
    assert!(commands.iter().any(|c| c.contains("RCPT TO")));
    assert!(commands.iter().any(|c| c == "DATA"));

    let written = connector.log().written;
    assert!(
        written
            .windows(MESSAGE.len())
            .any(|window| window == MESSAGE),
        "the message body reached the wire"
    );
}

#[tokio::test]
async fn several_recipients_each_get_their_own_rcpt_to() {
    let (mut session, connector) = open(happy_script()).await;

    session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned(), "bob@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap();

    let rcpt_count = connector
        .log()
        .commands()
        .iter()
        .filter(|c| c.contains("RCPT TO"))
        .count();
    assert_eq!(rcpt_count, 2);
}

#[tokio::test]
async fn quit_closes_the_session_politely() {
    let (session, connector) = open(happy_script().on("QUIT", "221 bye")).await;

    session.quit().await.expect("QUIT");

    assert!(connector.log().commands().iter().any(|c| c == "QUIT"));
}

// ---------------------------------------------------------------------------
// STARTTLS on the submission port
// ---------------------------------------------------------------------------

#[tokio::test]
async fn starttls_reissues_ehlo_over_the_upgraded_connection() {
    let script = SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 STARTTLS")
        .on("STARTTLS", "220 go ahead");
    let connector = ScriptedConnector::new(script);

    let settings = ConnectionSettings::new(
        "smtp.example.com",
        SUBMISSION_PORT,
        TransportSecurity::StartTls,
        "ada@example.com",
    );

    // The post-upgrade EHLO capabilities carry no working AUTH mechanism in
    // this transcript, so the session still fails to open overall — but not
    // before proving the upgrade and the second EHLO happened.
    SmtpSession::open(&settings, &password(), &connector)
        .await
        .unwrap_err();

    let commands = connector.log().commands();
    let ehlo_count = commands.iter().filter(|c| c.starts_with("EHLO")).count();
    assert_eq!(ehlo_count, 2, "expected EHLO before and after STARTTLS");
    assert!(commands.iter().any(|c| c == "STARTTLS"));
    assert_eq!(connector.log().upgrades, vec!["smtp.example.com"]);
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_password_is_an_authentication_failure_not_a_retry() {
    let script = SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "535 5.7.8 authentication credentials invalid");
    let connector = ScriptedConnector::new(script);

    let error = SmtpSession::open(&settings(), &password(), &connector)
        .await
        .unwrap_err();

    assert!(error.is_authentication_failure());
    assert!(!error.is_transient());
    assert!(error.to_string().contains("535"));
}

#[tokio::test]
async fn a_temporary_auth_failure_is_transient_not_a_wrong_password() {
    let script = SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "454 4.7.0 temporary authentication failure");
    let connector = ScriptedConnector::new(script);

    let error = SmtpSession::open(&settings(), &password(), &connector)
        .await
        .unwrap_err();

    assert!(error.is_transient());
    assert!(!error.is_authentication_failure());
}

// ---------------------------------------------------------------------------
// Sender and recipient rejection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_permanently_rejected_sender_is_distinguishable_from_a_recipient_problem() {
    let script = script_overriding("MAIL FROM", "550 5.1.8 sender address rejected");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::SenderRejected { .. }));
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_permanently_rejected_recipient_names_the_recipient() {
    let script = script_overriding("RCPT TO", "550 5.1.1 mailbox unavailable");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["nobody@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::RecipientRejected { .. }));
    assert_eq!(error.rejected_recipient(), Some("nobody@example.com"));
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_temporarily_unavailable_recipient_is_transient_not_a_permanent_rejection() {
    // 450 is "mailbox busy, try later" — not the same as "this address will
    // never work," and must not be reported as a rejection to the user.
    let script = script_overriding("RCPT TO", "450 4.2.1 mailbox busy");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(error.is_transient());
    assert_eq!(error.rejected_recipient(), None);
}

// ---------------------------------------------------------------------------
// The message body
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_message_body_is_distinguishable_from_a_rejected_recipient() {
    let script = happy_script().on_data_body("552 5.2.3 message too large");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::MessageRejected { .. }));
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_rate_limited_data_command_is_retried_with_backoff_not_dropped() {
    let script = script_overriding("DATA", "421 4.7.0 too many messages, slow down");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(error.is_transient(), "a 4xx must be retried, not dropped");
}

// ---------------------------------------------------------------------------
// Addresses and cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_address_that_could_inject_a_command_is_rejected_before_the_wire() {
    let (mut session, connector) = open(happy_script()).await;

    let error = session
        .send_message(
            "ada@example.com\r\nRCPT TO:<attacker@evil.test>",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::Configuration { .. }));
    assert!(
        connector
            .log()
            .commands()
            .iter()
            .all(|c| !c.contains("MAIL FROM")),
        "a malformed sender must never reach MAIL FROM"
    );
}

#[tokio::test]
async fn a_cancelled_token_stops_before_the_transaction_starts() {
    let (mut session, connector) = open(happy_script()).await;
    let cancel = CancelToken::new();
    cancel.cancel();

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &cancel,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::Cancelled));
    assert!(
        connector
            .log()
            .commands()
            .iter()
            .all(|c| !c.contains("MAIL FROM"))
    );
}

#[tokio::test]
async fn sending_to_no_recipients_is_a_configuration_error_not_a_bare_mail_from() {
    let (mut session, connector) = open(happy_script()).await;

    let error = session
        .send_message("ada@example.com", &[], MESSAGE, &CancelToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, SmtpError::Configuration { .. }));
    assert!(
        connector
            .log()
            .commands()
            .iter()
            .all(|c| !c.contains("MAIL FROM"))
    );
}

// ---------------------------------------------------------------------------
// Live server. Ignored by default; needs a real account.
// ---------------------------------------------------------------------------

/// Reads the live-test credentials, or skips.
///
/// `POSTIO_TEST_SMTP_USER` and `POSTIO_TEST_SMTP_PASSWORD` — for a provider
/// that requires one, an app-specific password, the same one the live IMAP
/// tests use since iCloud shares it across both protocols.
/// `POSTIO_TEST_SMTP_HOST` overrides the default of iCloud's submission host.
fn live_settings() -> Option<(ConnectionSettings, SecretString)> {
    let user = std::env::var("POSTIO_TEST_SMTP_USER").ok()?;
    let password = std::env::var("POSTIO_TEST_SMTP_PASSWORD").ok()?;
    let host =
        std::env::var("POSTIO_TEST_SMTP_HOST").unwrap_or_else(|_| "smtp.mail.me.com".to_owned());

    let settings = ConnectionSettings::new(host, SMTPS_PORT, TransportSecurity::Tls, &user);
    Some((settings, SecretString::from(password)))
}

#[tokio::test]
#[ignore = "talks to a live SMTP server; set POSTIO_TEST_SMTP_USER and POSTIO_TEST_SMTP_PASSWORD"]
async fn live_server_sends_a_message_to_self() {
    let Some((settings, password)) = live_settings() else {
        panic!("POSTIO_TEST_SMTP_USER and POSTIO_TEST_SMTP_PASSWORD must be set");
    };

    let connector = RustlsConnector::new().expect("TLS configuration");
    let mut session = SmtpSession::open(&settings, &password, &connector)
        .await
        .expect("live connect");
    assert!(session.is_encrypted());

    let address = settings.username.clone();
    let message = format!(
        "From: {address}\r\nTo: {address}\r\nSubject: Postio live SMTP send test\r\n\r\n\
         This is postio-t98's live send-to-self test.\r\n"
    );

    // This proves delivery was accepted, not that the sent copy landed in
    // the Sent folder: that needs an IMAP APPEND, which this crate has no
    // dependency to perform. Wiring the two together is postio-pzy's job.
    session
        .send_message(
            &address,
            std::slice::from_ref(&address),
            message.as_bytes(),
            &postio_smtp::cancel::CancelToken::new(),
        )
        .await
        .expect("live send");

    session.quit().await.expect("quit");
}

// ---------------------------------------------------------------------------
// Whether the payload may already have gone (ADR 0021)
// ---------------------------------------------------------------------------

/// The distinction `is_transient` cannot make and a send has to have.
///
/// A `4xx` is a *reply*: the server was still talking and did not accept the
/// message, so the transaction ended in a refusal the client witnessed and
/// the send is safely retryable. That it is also transient is a different
/// fact about a different question.
#[tokio::test]
async fn a_rate_limited_data_command_did_not_deliver_anything() {
    let script = script_overriding("DATA", "421 4.7.0 too many messages, slow down");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(error.is_transient());
    assert!(
        !error.submission_is_indeterminate(),
        "the server answered, so the client knows the message was not taken"
    );
}

#[tokio::test]
async fn a_rejected_recipient_did_not_deliver_anything_either() {
    let script = script_overriding("RCPT TO", "550 5.1.1 no such user");
    let (mut session, _connector) = open(script).await;

    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();

    assert!(!error.submission_is_indeterminate());
}

// ---------------------------------------------------------------------------
// Where the payload begins — ADR 0021, #673
// ---------------------------------------------------------------------------

/// Sends `MESSAGE` over a connector that dies somewhere, and hands back the
/// error and the connector so a test can also ask what actually went out.
async fn send_over(connector: ScriptedConnector) -> (SmtpError, ScriptedConnector) {
    let mut session = SmtpSession::open(&settings(), &password(), &connector)
        .await
        .expect("the scripted handshake");
    let error = session
        .send_message(
            "ada@example.com",
            &["grace@example.com".to_owned()],
            MESSAGE,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
    (error, connector)
}

#[tokio::test]
async fn a_connection_lost_before_mail_from_submitted_nothing() {
    // The safe half. Nothing of the message reached the server, so the send
    // is an ordinary transient failure and the drainer should try it again --
    // refusing to would strand a queued message on a connection hiccup.
    let (error, connector) =
        send_over(ScriptedConnector::new(happy_script()).vanishing_at("MAIL FROM")).await;

    assert!(error.is_transient(), "{error}");
    assert!(
        !error.submission_is_indeterminate(),
        "nothing of the message was written, so retrying cannot deliver it \
         twice: {error}"
    );
    let commands = connector.log().commands();
    assert!(
        !commands.iter().any(|line| line.starts_with("MAIL FROM")),
        "the fixture is wrong if MAIL FROM reached the server: {commands:?}"
    );
}

#[tokio::test]
async fn the_data_command_itself_is_still_before_the_payload() {
    // The sharp edge. `DATA` is a four-letter command that asks permission;
    // the message goes out only after the server's `354`. A connection lost
    // between the two has submitted nothing, and treating it as unknowable
    // would strand every message that hit a hiccup at exactly that moment.
    let (error, connector) =
        send_over(ScriptedConnector::new(happy_script()).vanishing_at("DATA")).await;

    assert!(error.is_transient(), "{error}");
    assert!(
        !error.submission_is_indeterminate(),
        "DATA asks permission; it does not submit anything: {error}"
    );
    assert!(
        !connector.log().written.ends_with(b"\r\n.\r\n"),
        "no payload should have been written"
    );
}

#[tokio::test]
async fn a_connection_lost_once_the_payload_is_on_the_wire_is_unknowable() {
    // The dangerous half, and the whole reason ADR 0021 exists: the reply to
    // the terminating `.` is exactly what is missing, so "it never arrived"
    // and "it arrived and the acknowledgement did not" look identical from
    // here. A retry may deliver a second copy to somebody else's inbox.
    let (error, connector) =
        send_over(ScriptedConnector::new(happy_script()).vanishing_after_the_payload()).await;

    assert!(
        error.submission_is_indeterminate(),
        "the payload was written and never answered; this must not be \
         retried automatically: {error}"
    );
    assert!(
        error.is_transient(),
        "the two predicates are orthogonal -- the connection failure is still \
         the retryable kind, which is precisely why the first one has to be \
         asked before it: {error}"
    );
    assert!(
        connector.log().written.ends_with(b"\r\n.\r\n"),
        "the fixture is wrong if the payload never went out"
    );
}

#[tokio::test]
async fn a_timeout_waiting_for_the_reply_to_the_terminator_is_unknowable_too() {
    // A timeout and a close are different errors and the send path has to
    // reach the same conclusion from both: the server may have taken it.
    let (error, _connector) = send_over(
        ScriptedConnector::new(happy_script())
            .timing_out_after_the_payload(std::time::Duration::from_secs(30)),
    )
    .await;

    assert!(
        error.submission_is_indeterminate(),
        "a server that goes quiet after the payload is the same problem \
         whether it closed or merely stopped talking: {error}"
    );
    assert!(error.is_transient(), "{error}");
}

#[tokio::test]
async fn a_message_the_server_refused_after_the_payload_is_not_unknowable() {
    // The other side of the payload boundary. The bytes went out *and the
    // server answered* -- with a refusal. It said what it did with the
    // message, so there is nothing indeterminate about it and the queue may
    // act on the refusal.
    let script = happy_script().on_data_body("552 5.3.4 message too big");
    let (error, _connector) = send_over(ScriptedConnector::new(script)).await;

    assert!(
        !error.submission_is_indeterminate(),
        "the server answered; being past the payload does not make a \
         witnessed refusal unknowable: {error}"
    );
    assert!(!error.is_transient(), "a 5xx is permanent: {error}");
}

#[test]
fn a_transport_failure_on_its_own_says_nothing_about_the_payload() {
    // The predicate is about *where* a failure happened, not which variant
    // it is. Before #673 this was approximated by variant, which meant every
    // dropped connection anywhere in the session was treated as though the
    // message might have gone out.
    for error in [
        SmtpError::Disconnected {
            context: "EHLO".to_owned(),
            reason: "connection reset".to_owned(),
        },
        SmtpError::TimedOut {
            context: "AUTH PLAIN".to_owned(),
            after: std::time::Duration::from_secs(30),
        },
        SmtpError::Io {
            context: "MAIL FROM".to_owned(),
            reason: "broken pipe".to_owned(),
        },
        SmtpError::Cancelled,
    ] {
        assert!(
            !error.submission_is_indeterminate(),
            "an untagged transport failure happened before the payload: {error}"
        );
    }
}
