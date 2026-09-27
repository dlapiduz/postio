//! Remote images for an allowed sender, end to end (spec 006 US5, FR-025,
//! T136), under #1336's two-direction discipline: every "nothing was
//! fetched" is paired with a control that proves the listener would have
//! seen it.
//!
//! One loopback listener serves an 8x8 magenta PNG and records every
//! request. The messages are stored with their senders, flagged, and read
//! in the Flagged view, where a row is one message (`cursor_preview`'s
//! reason); the allow list is a file this case owns.
//!
//! Nothing here reaches a real host: every remote URL names the listener,
//! and the offline case names a port nothing listens on.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::reader::RemoteImageAllowList;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, Store, test_support};

/// An 8x8 magenta PNG: a colour nothing else in the reader paints.
const MAGENTA_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 8, 0, 0, 0, 8, 8, 2, 0,
    0, 0, 75, 109, 41, 220, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 240, 31, 43, 98,
    24, 90, 18, 0, 209, 167, 127, 129, 119, 240, 160, 99, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];

/// A loopback server that answers every request with the PNG and keeps
/// what it was asked.
struct Listener {
    port: u16,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    fn start() -> Listener {
        let socket = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        socket.set_nonblocking(true).unwrap();
        let port = socket.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let (connections, requests, stop) = (
                Arc::clone(&connections),
                Arc::clone(&requests),
                Arc::clone(&stop),
            );
            move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = socket.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    connections.fetch_add(1, Ordering::Relaxed);
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&buffer[..n]),
                        }
                    }
                    requests
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&request).into_owned());
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        MAGENTA_PNG.len()
                    );
                    let _ = stream.write_all(MAGENTA_PNG);
                }
            }
        });
        Listener {
            port,
            connections,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The paths asked for so far.
    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| request.split_whitespace().nth(1).map(str::to_owned))
            .collect()
    }

    fn asked_for(&self, path: &str) -> bool {
        self.paths().iter().any(|asked| asked.ends_with(path))
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Store `html` from `from`, with its body, the way the backfill commits one.
async fn store(
    database: &Store,
    (account, mailbox): (AccountId, MailboxId),
    from: &str,
    subject: &str,
    html: &str,
    age: i64,
) -> MessageId {
    let connection = database.connect().await.expect("a connection");
    let repository = MessageRepository::new(&connection);
    let received = chrono::Utc::now() - chrono::Duration::minutes(age);
    let mut message = Message::new(account, mailbox, received);
    message.subject = Some(subject.to_owned());
    message.from = vec![EmailAddress::new(None::<&str>, from)];
    message.sync.body_state = BodyState::Full;
    let id = repository.create(&mut message).await.expect("a message");
    let stored = StoredBody {
        text: None,
        html: Some(html.to_owned()),
        headers: None,
        headers_truncated: false,
        encoding_problems: false,
    };
    repository
        .set_body(id, &stored, BodyState::Full)
        .await
        .expect("a body");
    id
}

/// Magenta pixels in the reader's snapshot: the image, painted.
fn magenta_in(window: &Window) -> usize {
    let Some(document) = window.reader().view().document() else {
        return 0;
    };
    let raster = postio_render::rasterize(&document);
    raster
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 230 && p[1] < 30 && p[2] > 230)
        .count()
}

/// Turn the loop for `duration`, spending all of it: long enough for a
/// fetch that should not happen to have happened.
///
/// POSTIO-FIXED-DEADLINE: nothing is waited *for*; the window is spent to
/// give a forbidden fetch every chance to show itself.
async fn spend(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn an_allowed_senders_images_arrive_and_nothing_else_does() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let mut listener = Listener::start();
        let port = listener.port;
        let at = |path: &str| format!("http://127.0.0.1:{port}/{path}");
        // A port nothing listens on: bound, read, and let go.
        let closed = {
            let socket = TcpListener::bind("127.0.0.1:0").unwrap();
            socket.local_addr().unwrap().port()
        };

        let database = test_support::memory().await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        let place = {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            (account.id, inbox)
        };
        let image = |path: &str| {
            format!(
                r#"<p>a message</p><img src="{}" width="80" height="80">"#,
                at(path)
            )
        };
        // The newest is left unflagged, for `cursor_preview`'s reason: the
        // opening folder has already reported it.
        store(
            &database,
            place,
            "news@example.com",
            "Unflagged",
            "<p>x</p>",
            0,
        )
        .await;
        let plain = store(
            &database,
            place,
            "ada@example.com",
            "Plain",
            "<p>no images</p>",
            1,
        )
        .await;
        let allowed = store(
            &database,
            place,
            "trusted@example.com",
            "Allowed",
            &image("allowed.png"),
            2,
        )
        .await;
        let stranger = store(
            &database,
            place,
            "stranger@example.org",
            "Stranger",
            &image("once.png"),
            3,
        )
        .await;
        let vectors = {
            let raw = postio_model::test_corpus::load("html-every-url-vector");
            let parsed = postio_model::mime::parse(raw.bytes());
            let html = parsed.body.html.expect("the fixture is HTML").replace(
                "https://beacon.example.com",
                &format!("http://127.0.0.1:{port}"),
            );
            store(
                &database,
                place,
                "beacons@abuse.example.com",
                "Vectors",
                &html,
                4,
            )
            .await
        };
        let offline = store(
            &database,
            place,
            "trusted@example.com",
            "Offline",
            &format!(
                r#"<p>offline words</p><img src="http://127.0.0.1:{closed}/gone.png" width="80" height="80">"#
            ),
            5,
        )
        .await;
        let mut sweep = Vec::new();
        for n in 0..10 {
            sweep.push(
                store(
                    &database,
                    place,
                    &format!("sweep{n}@example.com"),
                    &format!("Sweep {n}"),
                    &image(&format!("sweep{n}.png")),
                    10 + n,
                )
                .await,
            );
        }
        {
            let connection = database.connect().await.expect("a connection");
            connection
                .execute(
                    "UPDATE messages SET flagged = 1 WHERE id NOT IN \
                     (SELECT id FROM messages ORDER BY received_at DESC LIMIT 1)",
                    (),
                )
                .await
                .expect("the fixture writes");
        }

        // Every sender allowed but the stranger.
        let allowlist = state_dir.path().join("remote-images.ini");
        let mut list = RemoteImageAllowList::default();
        for sender in ["trusted@example.com", "beacons@abuse.example.com"]
            .into_iter()
            .map(str::to_owned)
            .chain((0..10).map(|n| format!("sweep{n}@example.com")))
        {
            list.allow(&sender);
        }
        list.save_to(&allowlist).expect("the allow list is written");

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, _events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );
        let window = Window::default();
        window.set_allowlist_path(&allowlist);
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        let list_view = window.list();
        assert!(
            settle_until(async || list_view.model().n_items() > 0).await,
            "the opening folder never filled"
        );
        wired
            .feeds
            .messages
            .open(postio_model::ListScope::Flagged(place.0));
        assert!(
            settle_until(async || list_view.model().n_items() == 15).await,
            "the Flagged view never filled"
        );
        let open = async |message: MessageId, words: &str| {
            list_view.select_message(message);
            assert!(
                settle_until(async || window
                    .reader()
                    .view()
                    .document()
                    .is_some_and(|d| d.text.text.contains(words)))
                .await,
                "{words:?} never reached the reader"
            );
        };

        // ── the control: the listener is reachable ───────────────────────
        drop(std::net::TcpStream::connect(("127.0.0.1", port)).expect("the control connects"));
        assert!(
            settle_until(async || listener.connections.load(Ordering::Relaxed) == 1).await,
            "the control connection was not counted, so every zero below is \
             meaningless"
        );

        // ── a cursor sweeping past ten allowed messages fetches nothing ──
        for message in &sweep {
            list_view.select_message(*message);
            spend(Duration::from_millis(60)).await;
        }
        open(plain, "no images").await;
        spend(postio_ui::dwell::DWELL_TO_READ * 2).await;
        assert!(
            !listener.paths().iter().any(|path| path.contains("sweep")),
            "a cursor passing over allowed senders' messages fetched their \
             images: {:?}",
            listener.paths()
        );

        // ── an allowed sender: placeholders first, then the image ────────
        open(allowed, "a message").await;
        assert_eq!(
            magenta_in(&window),
            0,
            "the first frame waited on the network instead of drawing \
             placeholders"
        );
        assert!(
            settle_until(async || listener.asked_for("/allowed.png")).await,
            "an allowed sender's opened message never fetched its image"
        );
        assert!(
            settle_until(async || magenta_in(&window) > 1_000).await,
            "the image arrived and was never painted"
        );
        let request = listener
            .requests
            .lock()
            .unwrap()
            .iter()
            .find(|request| request.contains("/allowed.png"))
            .cloned()
            .expect("the request was recorded")
            .to_ascii_lowercase();
        for header in ["cookie:", "referer:", "origin:", "authorization:"] {
            assert!(
                !request.contains(header),
                "the request carried {header}\n{request}"
            );
        }
        assert!(
            !request.contains("postio"),
            "the request names Postio\n{request}"
        );

        // ── a stranger: nothing, until "Show once", then that only ───────
        open(stranger, "a message").await;
        spend(postio_ui::dwell::DWELL_TO_READ * 2).await;
        assert!(
            !listener.asked_for("/once.png"),
            "a sender nobody allowed had their image fetched"
        );
        let before = listener.paths().len();
        window.reader().click_show_once();
        assert!(
            settle_until(async || listener.asked_for("/once.png")).await,
            "\"Show once\" fetched nothing"
        );
        spend(Duration::from_millis(300)).await;
        assert_eq!(
            listener.paths().len(),
            before + 1,
            "\"Show once\" fetched more than that message's image: {:?}",
            listener.paths()
        );

        // ── every URL vector: images and backgrounds, nothing else ───────
        open(vectors, "Background image.").await;
        assert!(
            settle_until(async || listener.asked_for("/pixel.gif")).await,
            "the allowed sender's <img> was never fetched, so the refusals \
             below prove nothing"
        );
        spend(Duration::from_millis(500)).await;
        for image in ["/bg.png", "/inline.png", "/cell.png", "/wide.png"] {
            assert!(
                listener.asked_for(image),
                "{image} is an image and was not fetched"
            );
        }
        for refused in [
            "/import.css",
            "/font.woff2",
            "/bullet.png",
            "/cursor.png",
            "/border.png",
            "/content.png",
        ] {
            assert!(
                !listener.asked_for(refused),
                "{refused} is not an image source and was fetched"
            );
        }

        // ── offline: the message is drawn at once, placeholders and all ──
        listener.close();
        let started = std::time::Instant::now();
        open(offline, "offline words").await;
        assert!(
            started.elapsed() < postio_test_support::scaled(Duration::from_secs(2)),
            "an unreachable image held the message back for {:?}",
            started.elapsed()
        );
        spend(postio_ui::dwell::DWELL_TO_READ * 2).await;
        assert!(
            window
                .reader()
                .view()
                .document()
                .is_some_and(|d| d.text.text.contains("offline words")),
            "a failed fetch took the message off the screen"
        );

        bridge.shutdown();
    });
}
