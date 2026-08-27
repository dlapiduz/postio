//! The adapter against a scripted Gmail REST server on loopback (#546).
//!
//! Each test states what the server answers per method+path prefix, and
//! the assertions read both what came back through the seam and what the
//! adapter sent. Nothing touches the network beyond 127.0.0.1.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use postio_gmail::GmailBackend;
use postio_imap::backend::{
    AppendMessage, BodyPart, FlagChange, MailBackend, MailboxFilter, VecSink,
};
use postio_imap::cancel::CancelToken;
use postio_model::{Flag, FlagSet, MailboxRole, RemoteId, Uid};

/// `(method, path-prefix, response-json)` rules, matched in order; a rule
/// is consumed when matched.
struct ScriptedServer {
    port: u16,
    rules: Arc<Mutex<VecDeque<(String, String, String)>>>,
    requests: Arc<Mutex<Vec<String>>>,
    _accept: thread::JoinHandle<()>,
}

impl ScriptedServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let rules: Arc<Mutex<VecDeque<(String, String, String)>>> = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();

        let serve_rules = rules.clone();
        let serve_requests = requests.clone();
        let accept = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let request_line = line.trim_end().to_owned();
                    if request_line.is_empty() {
                        continue;
                    }
                    let mut content_length = 0usize;
                    let mut authorized = false;
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            break;
                        }
                        let header = line.trim_end().to_ascii_lowercase();
                        if let Some(value) = header
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse().ok())
                        {
                            content_length = value;
                        }
                        if header.starts_with("authorization: bearer ") {
                            authorized = true;
                        }
                        if line.trim_end().is_empty() {
                            break;
                        }
                    }
                    let mut body = vec![0u8; content_length];
                    let _ = reader.read_exact(&mut body);
                    let body = String::from_utf8_lossy(&body).into_owned();
                    serve_requests
                        .lock()
                        .expect("requests")
                        .push(format!("{request_line}\n{body}"));

                    let mut parts = request_line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_owned();
                    let path = parts.next().unwrap_or_default().to_owned();

                    let canned = if authorized {
                        let mut rules = serve_rules.lock().expect("rules");
                        let index = rules.iter().position(|(m, prefix, _)| {
                            *m == method && path.starts_with(prefix.as_str())
                        });
                        index.map(|index| rules.remove(index).expect("indexed").2)
                    } else {
                        None
                    };
                    let response = match (authorized, canned) {
                        (false, _) => plain("401 Unauthorized", b"{}"),
                        (true, Some(json)) => plain("200 OK", json.as_bytes()),
                        (true, None) => plain(
                            "500 Internal Server Error",
                            format!("nothing scripted for {method} {path}").as_bytes(),
                        ),
                    };
                    if stream.write_all(&response).is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            port,
            rules,
            requests,
            _accept: accept,
        }
    }

    fn on(&self, method: &str, prefix: &str, json: &str) -> &Self {
        self.rules.lock().expect("rules").push_back((
            method.to_owned(),
            prefix.to_owned(),
            json.to_owned(),
        ));
        self
    }

    fn backend(&self) -> GmailBackend {
        GmailBackend::new("test-token").with_loopback_endpoint("127.0.0.1", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

fn plain(status: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

const LABELS: &str = r#"{"labels": [
    {"id": "INBOX", "name": "INBOX", "type": "system"},
    {"id": "SENT", "name": "SENT", "type": "system"},
    {"id": "DRAFT", "name": "DRAFT", "type": "system"},
    {"id": "TRASH", "name": "TRASH", "type": "system"},
    {"id": "SPAM", "name": "SPAM", "type": "system"},
    {"id": "Label_7", "name": "Receipts", "type": "user"}
]}"#;

#[tokio::test]
async fn labels_become_role_mailboxes_and_the_archive_exists() {
    let server = ScriptedServer::start();
    server.on("GET", "/gmail/v1/users/me/labels", LABELS);
    let backend = server.backend();

    let mailboxes = backend
        .list_mailboxes(&MailboxFilter::default())
        .await
        .expect("list");

    let by_path = |path: &str| {
        mailboxes
            .iter()
            .find(|mailbox| mailbox.path == path)
            .unwrap_or_else(|| panic!("no {path} in {mailboxes:?}"))
    };
    assert_eq!(by_path("Inbox").role, MailboxRole::Inbox);
    assert_eq!(by_path("Sent").role, MailboxRole::Sent);
    assert_eq!(by_path("Junk").role, MailboxRole::Junk);
    assert_eq!(
        by_path("Archive").role,
        MailboxRole::Archive,
        "archive exists even though Gmail has no label for it: it is the \
         destination the archive verb needs"
    );
    assert!(
        mailboxes.iter().any(|mailbox| mailbox.path == "Receipts"),
        "user labels surface as plain folders"
    );
}

#[tokio::test]
async fn fetched_headers_carry_the_id_verbatim_and_seen_is_inverted_unread() {
    let server = ScriptedServer::start();
    server.on("GET", "/gmail/v1/users/me/labels", LABELS);
    server.on(
        "GET",
        "/gmail/v1/users/me/messages?",
        r#"{"messages": [{"id": "gm-2"}, {"id": "gm-1"}], "resultSizeEstimate": 2}"#,
    );
    server.on(
        "GET",
        "/gmail/v1/users/me/messages/gm-1",
        r#"{"id": "gm-1", "labelIds": ["INBOX", "UNREAD"], "internalDate": "1755680000000",
            "sizeEstimate": 100, "payload": {"headers": [
              {"name": "Subject", "value": "First"},
              {"name": "From", "value": "Ada Lovelace <ada@example.com>"},
              {"name": "Message-ID", "value": "<first@example.com>"}]}}"#,
    );
    server.on(
        "GET",
        "/gmail/v1/users/me/messages/gm-2",
        r#"{"id": "gm-2", "labelIds": ["INBOX", "STARRED"], "internalDate": "1755770000000",
            "sizeEstimate": 200, "payload": {"headers": [
              {"name": "Subject", "value": "Second"}]}}"#,
    );
    let backend = server.backend();

    let set = [1, 2].into_iter().map(Uid::new).collect();
    let fetched = backend
        .fetch_headers("Inbox", &set, None, &CancelToken::new())
        .await
        .expect("fetch");

    assert_eq!(fetched.len(), 2);
    assert_eq!(
        fetched[0].remote_id,
        RemoteId::new("gm-1"),
        "position 1 is the oldest; the identity is the Gmail id verbatim"
    );
    assert!(!fetched[0].flags.is_seen(), "UNREAD present means not seen");
    assert!(
        fetched[1].flags.is_seen() && fetched[1].flags.is_flagged(),
        "no UNREAD means seen; STARRED means flagged: {:?}",
        fetched[1].flags
    );
    let envelope = fetched[0].envelope.as_ref().expect("envelope");
    assert_eq!(envelope.subject.as_deref(), Some("First"));
    assert_eq!(envelope.from[0].address, "ada@example.com");
    assert_eq!(envelope.from[0].name.as_deref(), Some("Ada Lovelace"));
}

#[tokio::test]
async fn marking_seen_removes_unread_and_reports_the_labels_truth() {
    let server = ScriptedServer::start();
    server.on(
        "POST",
        "/gmail/v1/users/me/messages/gm-1/modify",
        r#"{"id": "gm-1", "labelIds": ["INBOX"]}"#,
    );
    server.on(
        "GET",
        "/gmail/v1/users/me/messages/gm-1",
        r#"{"id": "gm-1", "labelIds": ["INBOX"]}"#,
    );
    let backend = server.backend();

    let updates = backend
        .store_flags(
            "Inbox",
            &[RemoteId::new("gm-1")],
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("store");

    assert_eq!(updates.len(), 1);
    assert!(updates[0].flags.is_seen());
    let modify = server
        .requests()
        .iter()
        .find(|request| request.contains("/modify"))
        .cloned()
        .expect("the modify request");
    assert!(
        modify.contains("removeLabelIds") && modify.contains("UNREAD"),
        "seen travels as removing UNREAD: {modify}"
    );
}

#[tokio::test]
async fn archiving_removes_the_inbox_label_and_adds_nothing() {
    let server = ScriptedServer::start();
    server.on("GET", "/gmail/v1/users/me/labels", LABELS);
    server.on(
        "POST",
        "/gmail/v1/users/me/messages/gm-1/modify",
        r#"{"id": "gm-1", "labelIds": []}"#,
    );
    let backend = server.backend();

    let mapping = backend
        .move_messages("Inbox", &[RemoteId::new("gm-1")], "Archive")
        .await
        .expect("archive");

    assert_eq!(mapping[0].destination_remote_id(), RemoteId::new("gm-1"));
    let modify = server
        .requests()
        .iter()
        .find(|request| request.contains("/modify"))
        .cloned()
        .expect("the modify request");
    assert!(
        modify.contains(r#""removeLabelIds":["INBOX"]"#),
        "archive means remove-INBOX-label: {modify}"
    );
    assert!(
        !modify.contains("addLabelIds") || modify.contains(r#""addLabelIds":[]"#),
        "and nothing is added: {modify}"
    );
}

#[tokio::test]
async fn the_deleted_mark_is_the_trash_and_expunge_is_permanent_and_targeted() {
    let server = ScriptedServer::start();
    server.on(
        "POST",
        "/gmail/v1/users/me/messages/gm-1/trash",
        r#"{"id": "gm-1", "labelIds": ["TRASH"]}"#,
    );
    server.on("DELETE", "/gmail/v1/users/me/messages/gm-1", r#"{}"#);
    let backend = server.backend();

    let updates = backend
        .store_flags(
            "Inbox",
            &[RemoteId::new("gm-1")],
            &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("trash");
    assert_eq!(updates.len(), 1);

    let deleted = backend
        .expunge("Trash", Some(&[RemoteId::new("gm-1")]))
        .await
        .expect("expunge");
    assert_eq!(deleted, vec![RemoteId::new("gm-1")]);

    let untargeted = backend.expunge("Trash", None).await.expect("no-op");
    assert!(
        untargeted.is_empty(),
        "an untargeted expunge deletes nothing this adapter was not handed by id"
    );
}

#[tokio::test]
async fn an_append_inserts_the_raw_message_under_the_right_labels() {
    let server = ScriptedServer::start();
    server.on("GET", "/gmail/v1/users/me/labels", LABELS);
    server.on(
        "POST",
        "/gmail/v1/users/me/messages",
        r#"{"id": "gm-new", "labelIds": ["SENT"]}"#,
    );
    let backend = server.backend();

    let mapping = backend
        .append(
            "Sent",
            &AppendMessage::new(b"Subject: hi\r\n\r\nx".to_vec())
                .with_flags(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("append")
        .expect("the created id comes back");

    assert_eq!(mapping.destination_remote_id(), RemoteId::new("gm-new"));
    let insert = server
        .requests()
        .iter()
        .find(|request| request.contains("POST /gmail/v1/users/me/messages"))
        .cloned()
        .expect("the insert request");
    assert!(
        insert.contains("SENT") && !insert.contains("UNREAD"),
        "a seen append carries its label and no UNREAD: {insert}"
    );
}

#[tokio::test]
async fn a_whole_body_arrives_from_the_raw_format() {
    let server = ScriptedServer::start();
    let raw = io_gmail::v1::rest::messages::encode_raw(
        b"Return-Path: <ada@example.com>\r\nSubject: hi\r\n\r\nthe body",
    );
    server.on(
        "GET",
        "/gmail/v1/users/me/messages/gm-1",
        &format!(r#"{{"id": "gm-1", "raw": "{raw}"}}"#),
    );
    let backend = server.backend();

    let mut sink = VecSink::new();
    let fetched = backend
        .fetch_part(
            "Inbox",
            &RemoteId::new("gm-1"),
            &BodyPart::Whole,
            &mut sink,
            &CancelToken::new(),
        )
        .await
        .expect("fetch the body");

    assert_eq!(fetched.remote_id, RemoteId::new("gm-1"));
    assert!(sink.into_inner().starts_with(b"Return-Path:"));
}

#[tokio::test]
async fn find_by_message_id_asks_gmails_own_search() {
    let server = ScriptedServer::start();
    server.on(
        "GET",
        "/gmail/v1/users/me/messages?",
        r#"{"messages": [{"id": "gm-found"}], "resultSizeEstimate": 1}"#,
    );
    let backend = server.backend();

    let found = backend
        .find_by_message_id("Sent", "<hello@example.com>")
        .await
        .expect("search");
    assert_eq!(found, Some(RemoteId::new("gm-found")));
    assert!(
        server.requests()[0].contains("rfc822msgid"),
        "{:?}",
        server.requests()
    );
}
