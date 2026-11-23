use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use postio_dav::{CalendarBackend, DavError, WebdavBackend};

const EVENT: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VEVENT\r\n\
UID:a@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20261201T100000Z\r\nSUMMARY:A\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

fn multistatus(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:multistatus xmlns:d=\"DAV:\" \
         xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:a=\"http://apple.com/ns/ical/\">\
         {body}</d:multistatus>"
    )
}

fn found(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop>\
         <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn answer(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &str,
) -> (u16, Vec<(&'static str, String)>, String) {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    match (method, path) {
        ("PROPFIND", "/") => (207, vec![], multistatus(&found("/", "<d:current-user-principal><d:href>/principals/ada/</d:href></d:current-user-principal>"))),
        ("PROPFIND", "/principals/ada/") => (207, vec![], multistatus(&found("/principals/ada/", "<c:calendar-home-set><d:href>/cal/ada/</d:href></c:calendar-home-set>"))),
        ("PROPFIND", "/cal/ada/") => (
            207,
            vec![],
            multistatus(&[
                found("/cal/ada/", "<d:resourcetype><d:collection/></d:resourcetype>"),
                found("/cal/ada/work/", "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Work</d:displayname><a:calendar-color>#3584e4</a:calendar-color><c:supported-calendar-component-set><c:comp name=\"VEVENT\"/></c:supported-calendar-component-set>"),
                found("/cal/ada/tasks/", "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname><c:supported-calendar-component-set><c:comp name=\"VTODO\"/></c:supported-calendar-component-set>"),
            ]
            .concat()),
        ),
        ("REPORT", "/cal/ada/work/") if body.contains("sync-collection") => {
            if body.contains("tok-expired") {
                (403, vec![], "<?xml version=\"1.0\"?><d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>".to_owned())
            } else if body.contains("tok-1") {
                (207, vec![], multistatus(&format!(
                    "{}<d:response><d:href>/cal/ada/work/gone.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response><d:sync-token>tok-2</d:sync-token>",
                    found("/cal/ada/work/a.ics", "<d:getetag>\"e2\"</d:getetag>")
                )))
            } else {
                (207, vec![], multistatus(&format!("{}<d:sync-token>tok-1</d:sync-token>", found("/cal/ada/work/a.ics", "<d:getetag>\"e1\"</d:getetag>"))))
            }
        }
        ("REPORT", "/cal/ada/work/") if body.contains("calendar-multiget") => (207, vec![], multistatus(&found(
            "/cal/ada/work/a.ics",
            &format!("<d:getetag>\"e1\"</d:getetag><c:calendar-data>{}</c:calendar-data>", EVENT.replace("\r\n", "&#13;\n")),
        ))),
        ("REPORT", "/cal/ada/work/") if body.contains("calendar-query") => {
            let windowed = body.contains("time-range start=\"20261123T000000Z\"");
            let mut rows = found("/cal/ada/work/a.ics", "<d:getetag>\"e1\"</d:getetag>");
            if !windowed {
                rows.push_str(&found("/cal/ada/work/old.ics", "<d:getetag>\"o1\"</d:getetag>"));
            }
            (207, vec![], multistatus(&rows))
        }
        ("PUT", "/cal/ada/work/a.ics") => match header("if-match").as_deref() {
            Some("\"e1\"") => (204, vec![("ETag", "\"e3\"".to_owned())], String::new()),
            _ => (412, vec![], String::new()),
        },
        ("PUT", "/cal/ada/work/new.ics") if header("if-none-match").as_deref() == Some("*") => {
            (201, vec![("ETag", "\"n1\"".to_owned())], String::new())
        }
        ("PUT", "/cal/ada/holidays/h.ics") => (403, vec![], String::new()),
        ("DELETE", "/cal/ada/work/a.ics") => (204, vec![], String::new()),
        _ => (404, vec![], String::new()),
    }
}

struct Server {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let heard = Arc::clone(&log);
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(connection) = connection else { continue };
            let heard = Arc::clone(&heard);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(connection.try_clone().expect("a clone"));
                let mut out = connection;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let (method, path) = (
                        parts.next().unwrap_or_default().to_owned(),
                        parts.next().unwrap_or_default().to_owned(),
                    );
                    let mut headers = Vec::new();
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).expect("a header");
                        let header = header.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        let (key, value) = header.split_once(':').expect("a header line");
                        headers.push((key.trim().to_owned(), value.trim().to_owned()));
                    }
                    let length: usize = headers
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .map_or(0, |(_, value)| value.parse().expect("a length"));
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).expect("a body");
                    heard
                        .lock()
                        .expect("the log")
                        .push(format!("{method} {path}"));
                    let (status, extra, text) =
                        answer(&method, &path, &headers, &String::from_utf8_lossy(&body));
                    let mut response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/xml; charset=utf-8\r\nContent-Length: {}\r\n",
                        text.len()
                    );
                    for (key, value) in extra {
                        response.push_str(&format!("{key}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    response.push_str(&text);
                    if out.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    Server { port, log }
}

fn backend(server: &Server) -> WebdavBackend {
    WebdavBackend::new(
        &format!("http://127.0.0.1:{}/", server.port),
        "ada",
        "app-password",
    )
    .expect("a backend")
}

#[tokio::test(flavor = "current_thread")]
async fn discovery_lists_the_event_calendars_and_reports_one_connection() {
    let server = server();
    let backend = backend(&server);

    let calendars = backend.calendars().await.expect("calendars");

    assert_eq!(
        calendars.len(),
        1,
        "a tasks-only collection is left out: {calendars:?}"
    );
    assert_eq!(
        (calendars[0].href.as_str(), calendars[0].name.as_str()),
        ("work", "Work")
    );
    assert_eq!(calendars[0].colour.as_deref(), Some("#3584e4"));
    backend.listing("work", None).await.expect("listed");
    let connections = backend.connections();
    assert_eq!(connections.len(), 1, "one connection serves both calls");
    assert_eq!(connections[0].host, "127.0.0.1");
    assert!(connections[0].reached);
    let heard = server.log.lock().expect("the log").clone();
    assert_eq!(
        &heard[..3],
        [
            "PROPFIND /",
            "PROPFIND /principals/ada/",
            "PROPFIND /cal/ada/"
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_window_asks_the_server_for_its_range_and_fetch_brings_the_bytes() {
    let server = server();
    let backend = backend(&server);
    let from = Utc.with_ymd_and_hms(2026, 11, 23, 0, 0, 0).unwrap();
    let until = Utc.with_ymd_and_hms(2028, 11, 23, 0, 0, 0).unwrap();

    let windowed = backend
        .listing("work", Some((from, until)))
        .await
        .expect("listed");
    let all = backend.listing("work", None).await.expect("listed");
    assert_eq!(
        windowed.iter().map(|l| l.href.as_str()).collect::<Vec<_>>(),
        ["a.ics"]
    );
    assert_eq!(all.len(), 2);
    assert_eq!(windowed[0].etag, "e1");

    let fetched = backend
        .fetch("work", &["a.ics".to_owned()])
        .await
        .expect("fetched");
    assert_eq!(fetched[0].ics, EVENT.as_bytes(), "the object byte for byte");
    assert_eq!(fetched[0].etag, "e1");
}

#[tokio::test(flavor = "current_thread")]
async fn changes_by_token_name_what_moved_and_what_went_and_an_expired_token_says_so() {
    let server = server();
    let backend = backend(&server);

    let first = backend.changes("work", None).await.expect("changes");
    assert_eq!(first.token.as_deref(), Some("tok-1"));
    let next = backend
        .changes("work", Some("tok-1"))
        .await
        .expect("changes");
    assert_eq!(
        next.changed
            .iter()
            .map(|l| (l.href.as_str(), l.etag.as_str()))
            .collect::<Vec<_>>(),
        [("a.ics", "e2")]
    );
    assert_eq!(next.removed, ["gone.ics"]);
    assert_eq!(next.token.as_deref(), Some("tok-2"));

    assert_eq!(
        backend.changes("work", Some("tok-expired")).await,
        Err(DavError::TokenExpired)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_put_is_conditional_and_its_refusals_say_what_sync_must_do() {
    let server = server();
    let backend = backend(&server);

    assert_eq!(
        backend
            .put("work", "a.ics", EVENT.as_bytes(), Some("e1"))
            .await,
        Ok("e3".to_owned())
    );
    assert_eq!(
        backend
            .put("work", "a.ics", EVENT.as_bytes(), Some("e0"))
            .await,
        Err(DavError::Changed)
    );
    assert_eq!(
        backend.put("work", "new.ics", EVENT.as_bytes(), None).await,
        Ok("n1".to_owned())
    );
    assert_eq!(
        backend
            .put("holidays", "h.ics", EVENT.as_bytes(), None)
            .await,
        Err(DavError::Forbidden)
    );
    assert_eq!(backend.delete("work", "a.ics", Some("e1")).await, Ok(()));
    assert_eq!(
        backend.delete("work", "missing.ics", Some("e1")).await,
        Ok(()),
        "gone already is done"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn an_unreachable_server_is_transient_and_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    drop(listener);
    let backend = WebdavBackend::new(&format!("http://127.0.0.1:{port}/"), "ada", "app-password")
        .expect("a backend");

    let error = backend.calendars().await.expect_err("nobody listens");
    assert!(error.is_transient(), "{error:?}");
    let connections = backend.connections();
    assert_eq!(connections.len(), 1);
    assert!(!connections[0].reached);
    assert!(
        backend.connections().is_empty(),
        "each connection is handed over once"
    );
}
