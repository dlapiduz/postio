//! The adapter against a scripted JMAP server on loopback (#544).
//!
//! The server is a thread answering canned JSON per method name — the
//! `SmtpScript` shape: each test states exactly what the server will say,
//! and the assertions read both what came back through the seam and what
//! the adapter actually sent. Nothing here touches the network beyond
//! 127.0.0.1.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use postio_imap::backend::{
    AppendMessage, BodyPart, FlagChange, MailBackend, MailboxFilter, SelectMode, VecSink,
};
use postio_imap::cancel::CancelToken;
use postio_jmap::JmapBackend;
use postio_model::{Flag, FlagSet, MailboxRole, RemoteId, Uid};

// --- the scripted server ------------------------------------------------

#[derive(Default)]
struct Script {
    /// Canned `methodResponses` bodies, popped per method name.
    api: HashMap<String, VecDeque<String>>,
    /// Bytes the download endpoint serves.
    blob: Vec<u8>,
}

struct ScriptedServer {
    port: u16,
    script: Arc<Mutex<Script>>,
    requests: Arc<Mutex<Vec<String>>>,
    _accept: thread::JoinHandle<()>,
}

impl ScriptedServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let script: Arc<Mutex<Script>> = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();

        let serve_script = script.clone();
        let serve_requests = requests.clone();
        let accept = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let request_line = line.trim_end().to_owned();
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
                        .map(str::trim)
                        .and_then(|value| value.parse().ok())
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

                let response = if !authorized {
                    plain("401 Unauthorized", b"{}")
                } else if request_line.contains("/session") {
                    plain("200 OK", session_json(port).as_bytes())
                } else if request_line.starts_with("POST /jmap/api") {
                    match method_of(&body).and_then(|method| {
                        serve_script
                            .lock()
                            .expect("script")
                            .api
                            .get_mut(&method)
                            .and_then(VecDeque::pop_front)
                    }) {
                        Some(canned) => plain("200 OK", canned.as_bytes()),
                        None => plain(
                            "500 Internal Server Error",
                            format!("nothing scripted for: {body}").as_bytes(),
                        ),
                    }
                } else if request_line.starts_with("POST /jmap/upload") {
                    plain(
                        "200 OK",
                        br#"{"accountId":"acc1","blobId":"blob-up-1","type":"message/rfc822","size":12}"#,
                    )
                } else if request_line.starts_with("GET /jmap/download") {
                    let bytes = serve_script.lock().expect("script").blob.clone();
                    plain("200 OK", &bytes)
                } else {
                    plain("404 Not Found", b"{}")
                };
                let _ = stream.write_all(&response);
            }
        });

        Self {
            port,
            script,
            requests,
            _accept: accept,
        }
    }

    fn on(&self, method: &str, body: &str) -> &Self {
        self.script
            .lock()
            .expect("script")
            .api
            .entry(method.to_owned())
            .or_default()
            .push_back(body.to_owned());
        self
    }

    fn blob(&self, bytes: &[u8]) -> &Self {
        self.script.lock().expect("script").blob = bytes.to_vec();
        self
    }

    fn backend(&self) -> JmapBackend {
        let url = format!("http://127.0.0.1:{}/jmap/session/", self.port)
            .parse()
            .expect("a session url");
        JmapBackend::new(url, "test-token")
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

fn plain(status: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn session_json(port: u16) -> String {
    format!(
        r#"{{
  "username": "ada@example.com",
  "accounts": {{
    "acc1": {{"name": "Ada", "isPersonal": true, "isReadOnly": false, "accountCapabilities": {{}}}}
  }},
  "primaryAccounts": {{"urn:ietf:params:jmap:core": "acc1", "urn:ietf:params:jmap:mail": "acc1"}},
  "capabilities": {{"urn:ietf:params:jmap:core": {{}}, "urn:ietf:params:jmap:mail": {{}}}},
  "apiUrl": "http://127.0.0.1:{port}/jmap/api/",
  "downloadUrl": "http://127.0.0.1:{port}/jmap/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}",
  "uploadUrl": "http://127.0.0.1:{port}/jmap/upload/{{accountId}}/",
  "eventSourceUrl": "http://127.0.0.1:{port}/jmap/eventsource/",
  "state": "s1"
}}"#
    )
}

/// The first method name in a JMAP request body.
fn method_of(body: &str) -> Option<String> {
    let calls = body.split("\"methodCalls\"").nth(1)?;
    let start = calls.find("[[\"")? + 3;
    let end = calls[start..].find('"')? + start;
    Some(calls[start..end].to_owned())
}

fn wrap(responses: &str) -> String {
    format!(r#"{{"methodResponses": [{responses}], "sessionState": "s1"}}"#)
}

const MAILBOXES: &str = r#"["Mailbox/get", {"state": "s1", "list": [
    {"id": "mb-inbox", "name": "Inbox", "role": "inbox", "totalEmails": 2, "unreadEmails": 1},
    {"id": "mb-archive", "name": "Archive", "role": "archive"},
    {"id": "mb-projects", "name": "Projects"},
    {"id": "mb-postio", "name": "Postio", "parentId": "mb-projects"}
  ], "notFound": []}, "c0"]"#;

// --- the tests ----------------------------------------------------------

#[tokio::test]
async fn the_session_capabilities_reach_the_seam_and_the_bearer_reaches_the_wire() {
    let server = ScriptedServer::start();
    let backend = server.backend();

    let capabilities = backend.connect().await.expect("connect");
    assert!(
        capabilities
            .names()
            .iter()
            .any(|name| name.contains("jmap:mail")),
        "{:?}",
        capabilities.names()
    );
    assert!(
        server.requests()[0].starts_with("GET /jmap/session"),
        "{:?}",
        server.requests()
    );
}

#[tokio::test]
async fn mailboxes_arrive_with_paths_assembled_and_roles_resolved() {
    let server = ScriptedServer::start();
    server.on("Mailbox/get", &wrap(MAILBOXES));
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let mailboxes = backend
        .list_mailboxes(&MailboxFilter::default())
        .await
        .expect("list");

    let inbox = mailboxes.iter().find(|m| m.path == "Inbox").expect("inbox");
    assert_eq!(inbox.role, MailboxRole::Inbox);
    let archive = mailboxes
        .iter()
        .find(|m| m.path == "Archive")
        .expect("archive");
    assert_eq!(archive.role, MailboxRole::Archive);
    assert!(
        mailboxes.iter().any(|m| m.path == "Projects/Postio"),
        "a child mailbox gets its parent in the path: {:?}",
        mailboxes.iter().map(|m| &m.path).collect::<Vec<_>>()
    );
}

const INBOX_EMAILS: &str = r#"["Email/query", {"accountId": "acc1", "queryState": "q1",
    "canCalculateChanges": false, "position": 0, "total": 2,
    "ids": ["em-1", "em-2"]}, "c0"],
  ["Email/get", {"accountId": "acc1", "state": "s1", "notFound": [], "list": [
    {"id": "em-1", "blobId": "blob-1", "size": 100,
     "receivedAt": "2026-08-20T09:31:00Z", "subject": "First",
     "from": [{"name": "Ada", "email": "ada@example.com"}],
     "keywords": {"$seen": true},
     "messageId": ["<first@example.com>"]},
    {"id": "em-2", "blobId": "blob-2", "size": 200,
     "receivedAt": "2026-08-21T10:00:00Z", "subject": "Second",
     "from": [{"name": "Grace", "email": "grace@example.net"}],
     "keywords": {}}
  ]}, "c1"]"#;

#[tokio::test]
async fn fetched_headers_carry_the_id_verbatim_and_a_position_for_a_uid() {
    let server = ScriptedServer::start();
    server.on("Mailbox/get", &wrap(MAILBOXES));
    server.on("Mailbox/get", &wrap(MAILBOXES));
    server.on("Email/query", &wrap(INBOX_EMAILS));
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let status = backend
        .select("Inbox", SelectMode::ReadWrite)
        .await
        .expect("select");
    assert_eq!(status.exists, 2);
    assert_eq!(status.uid_next, Uid::new(3), "positions run 1..=total");
    assert_eq!(
        status.highest_mod_seq, None,
        "no CondStore claim: resyncs re-enumerate"
    );

    let set = [1, 2].into_iter().map(Uid::new).collect();
    let fetched = backend
        .fetch_headers("Inbox", &set, None, &CancelToken::new())
        .await
        .expect("fetch");

    assert_eq!(fetched.len(), 2);
    assert_eq!(
        fetched[0].remote_id,
        RemoteId::new("em-1"),
        "the identity is the JMAP id verbatim — nothing packed"
    );
    assert_eq!(fetched[0].uid, Uid::new(1));
    assert!(fetched[0].flags.is_seen());
    let envelope = fetched[0].envelope.as_ref().expect("envelope");
    assert_eq!(envelope.subject.as_deref(), Some("First"));
    assert_eq!(envelope.from[0].address, "ada@example.com");
}

#[tokio::test]
async fn a_flag_change_patches_keywords_and_reports_the_servers_truth() {
    let server = ScriptedServer::start();
    server.on(
        "Email/set",
        &wrap(
            r#"["Email/set", {"accountId": "acc1", "newState": "s2",
            "created": {}, "updated": {"em-1": null}, "destroyed": []}, "c0"]"#,
        ),
    );
    server.on(
        "Email/get",
        &wrap(
            r#"["Email/get", {"accountId": "acc1", "state": "s2", "notFound": [], "list": [
            {"id": "em-1", "keywords": {"$seen": true, "$flagged": true}}]}, "c0"]"#,
        ),
    );
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let updates = backend
        .store_flags(
            "Inbox",
            &[RemoteId::new("em-1")],
            &FlagChange::Add(FlagSet::from_iter([Flag::Flagged])),
        )
        .await
        .expect("store");

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].remote_id, RemoteId::new("em-1"));
    assert!(updates[0].flags.is_flagged() && updates[0].flags.is_seen());

    let set_request = server
        .requests()
        .iter()
        .find(|request| request.contains("Email/set"))
        .cloned()
        .expect("the set request");
    assert!(
        set_request.contains("keywords/$flagged"),
        "the change travels as an RFC 7396 keyword patch: {set_request}"
    );
}

#[tokio::test]
async fn a_move_patches_mailbox_membership_and_the_id_survives() {
    let server = ScriptedServer::start();
    server.on("Mailbox/get", &wrap(MAILBOXES));
    server.on("Mailbox/get", &wrap(MAILBOXES));
    server.on(
        "Email/set",
        &wrap(
            r#"["Email/set", {"accountId": "acc1", "newState": "s2",
            "created": {}, "updated": {"em-1": null}, "destroyed": []}, "c0"]"#,
        ),
    );
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let mapping = backend
        .move_messages("Inbox", &[RemoteId::new("em-1")], "Archive")
        .await
        .expect("move");

    assert_eq!(mapping.len(), 1);
    assert_eq!(
        mapping[0].destination_remote_id(),
        RemoteId::new("em-1"),
        "a JMAP move keeps the id: same message, different membership"
    );
    let set_request = server
        .requests()
        .iter()
        .find(|request| request.contains("Email/set"))
        .cloned()
        .expect("the set request");
    assert!(
        set_request.contains("mailboxIds/mb-archive")
            && set_request.contains("mailboxIds/mb-inbox"),
        "{set_request}"
    );
}

#[tokio::test]
async fn an_append_uploads_the_blob_and_imports_it_where_it_was_asked() {
    let server = ScriptedServer::start();
    server.on("Mailbox/get", &wrap(MAILBOXES));
    server.on(
        "Email/import",
        &wrap(
            r#"["Email/import", {"accountId": "acc1", "newState": "s2",
            "created": {"postio-append": {"id": "em-new", "blobId": "blob-up-1"}},
            "notCreated": {}}, "c0"]"#,
        ),
    );
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let mapping = backend
        .append(
            "Archive",
            &AppendMessage::new(b"Subject: hi\r\n\r\nx".to_vec())
                .with_flags(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("append")
        .expect("the created id comes back");

    assert_eq!(mapping.destination_remote_id(), RemoteId::new("em-new"));
    let import = server
        .requests()
        .iter()
        .find(|request| request.contains("Email/import"))
        .cloned()
        .expect("the import request");
    assert!(
        import.contains("blob-up-1") && import.contains("mb-archive") && import.contains("$seen"),
        "{import}"
    );
}

#[tokio::test]
async fn a_targeted_expunge_destroys_exactly_what_it_was_handed() {
    let server = ScriptedServer::start();
    server.on(
        "Email/set",
        &wrap(
            r#"["Email/set", {"accountId": "acc1", "newState": "s2",
            "created": {}, "updated": {}, "destroyed": ["em-1"]}, "c0"]"#,
        ),
    );
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let destroyed = backend
        .expunge("Inbox", Some(&[RemoteId::new("em-1")]))
        .await
        .expect("expunge");

    assert_eq!(destroyed, vec![RemoteId::new("em-1")]);
}

#[tokio::test]
async fn a_whole_body_arrives_from_the_blob_download() {
    let server = ScriptedServer::start();
    server.blob(b"Return-Path: <ada@example.com>\r\nSubject: hi\r\n\r\nthe body");
    server.on(
        "Email/get",
        &wrap(
            r#"["Email/get", {"accountId": "acc1", "state": "s1", "notFound": [], "list": [
            {"id": "em-1", "blobId": "blob-1"}]}, "c0"]"#,
        ),
    );
    let backend = server.backend();
    backend.connect().await.expect("connect");

    let mut sink = VecSink::new();
    let fetched = backend
        .fetch_part(
            "Inbox",
            &RemoteId::new("em-1"),
            &BodyPart::Whole,
            &mut sink,
            &CancelToken::new(),
        )
        .await
        .expect("fetch the body");

    assert_eq!(fetched.remote_id, RemoteId::new("em-1"));
    assert!(sink.is_finished());
    assert!(sink.into_inner().starts_with(b"Return-Path:"));
}
