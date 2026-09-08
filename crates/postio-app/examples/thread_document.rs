//! ADR 0032's conversation, rendered into one `WebView`, so it can be looked at.
//!
//! The maintainer asked for a version to try (#1316): *"Can you create a
//! version of the app where the message reader is a single WebKit instance and
//! threads open inside it?"* This is that, standing on its own rather than
//! wired through the reading pane — the pane's body plumbing is per message
//! and deep, and swapping it is the next issue, not the one that answers
//! whether the idea is any good.
//!
//! ```sh
//! cargo run -p postio-app --example thread_document              # 8 messages
//! cargo run -p postio-app --example thread_document -- 50        # a long one
//! cargo run -p postio-app --example thread_document -- 1         # a single message
//! cargo run -p postio-app --example thread_document -- 50 --quiet   # measure only
//! ```
//!
//! It prints what ADR 0032's open questions ask for: how many web processes a
//! thread of this length costs, how much resident memory, and how long the
//! document took to hand over.
//!
//! Nothing here touches the network or reads anybody's mail: the messages are
//! written in this file, at reserved domains, so it runs the same on any
//! machine and can be shown to anyone.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::reader::view::{Reader, ThreadMessage};
use postio_gtk::{app, fonts, style};
use postio_model::message::MessageBody;

/// A thread that exercises what a real one does: prose, a quote, a reply
/// above it, an inline image reference, and one piece of bulk mail so the
/// per-message reader-view decision has something to decide.
fn thread(count: usize) -> Vec<ThreadMessage> {
    let people = [
        ("Ada Lovelace", "ada@example.com"),
        ("Grace Hopper", "grace@example.org"),
        ("Hedy Lamarr", "hedy@example.net"),
    ];
    (0..count)
        .map(|index| {
            let (sender, address) = people[index % people.len()];
            let last = index + 1 == count;
            let body = if index == 1 && count > 2 {
                // Bulk mail, so `suits_reader_view` has a case to answer.
                MessageBody {
                    html: Some(
                        "<table width=\"600\"><tr><td align=\"center\">\
                         <h1>Your order has shipped</h1>\
                         <p>Tracking TRK4820193776 &middot; arriving Thursday.</p>\
                         <p><img src=\"cid:logo\" alt=\"logo\"></p>\
                         <p><a href=\"https://example.com/track\">Track delivery</a></p>\
                         </td></tr></table>"
                            .into(),
                    ),
                    text: None,
                }
            } else {
                MessageBody {
                    html: Some(format!(
                        "<p>Message {n} of {count}. The maildir index rebuild walks every \
                         message on every open, which is why it is O(n\u{b2}) rather than \
                         merely slow.</p>\
                         <p>Some inline artwork: <img src=\"cid:figure-{n}\" alt=\"figure\"></p>\
                         <blockquote><p>On an earlier day, somebody wrote:</p>\
                         <p>Confirmed on 0.4.1 &mdash; the rebuild walks every file.</p>\
                         </blockquote>",
                        n = index + 1
                    )),
                    text: None,
                }
            };
            ThreadMessage {
                scope: index.to_string(),
                sender: sender.to_string(),
                address: address.to_string(),
                when: format!("{:02}:{:02}", 9 + index / 6, (index * 7) % 60),
                preview: format!(
                    "Message {} of {count} \u{2014} the rebuild walks every file\u{2026}",
                    index + 1
                ),
                // The newest opens, the rest are collapsed: the pane's own
                // opening policy is `conversation::expanded_on_open`, and
                // this is the shape it lands on for a read thread.
                expanded: last,
                latest: last,
                body,
            }
        })
        .collect()
}

/// This process's resident set, in KiB, straight out of `/proc`.
fn resident_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// The web processes this process owns.
///
/// Scoped to our own descendants by walking the parent chain, not by `pgrep
/// -P`: WebKit puts the web process under a `bwrap` sandbox, so it is a
/// grandchild rather than a child and `-P` finds nothing — which is a zero
/// that reads exactly like "one document costs no processes" and is a lie.
/// `gtk_reader` learned the same thing the same way.
///
/// `-x` against the **truncated** name, because Linux cuts `comm` to fifteen
/// characters: it is `WebKitWebProces`, and `-x WebKitWebProcess` matches
/// nothing while warning about it only on stderr.
fn web_processes() -> usize {
    let mine = std::process::id() as i32;
    std::process::Command::new("pgrep")
        .args(["-x", "WebKitWebProces"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<i32>().ok())
                .filter(|pid| descends_from(*pid, mine))
                .count()
        })
        .unwrap_or(0)
}

/// Whether `pid` is a descendant of `ancestor`, however deep the sandbox
/// nests it.
fn descends_from(pid: i32, ancestor: i32) -> bool {
    let mut current = pid;
    for _ in 0..16 {
        if current == ancestor {
            return true;
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{current}/status")) else {
            return false;
        };
        let Some(parent) = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .and_then(|value| value.trim().parse::<i32>().ok())
        else {
            return false;
        };
        if parent <= 1 {
            return false;
        }
        current = parent;
    }
    false
}

fn pump() {
    while glib::MainContext::default().iteration(false) {}
}

fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    // `--quiet` measures and exits, for scripting it; without it the window
    // stays up to be looked at, which is the point.
    let quiet = args.any(|arg| arg == "--quiet");

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("no display; this example needs one (scripts/test-headless.sh --status)");
        std::process::exit(1);
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let messages = thread(count);
    let window = gtk::Window::new();
    window.set_default_size(900, 900);
    window.set_title(Some(&format!("ADR 0032 — {count} messages, one document")));

    // No blobs: `cid:` references resolve to nothing and draw a broken image,
    // which is the honest result for a thread nobody has attachments for and
    // is what the corpus fixture `inline-image-cid` pins for the real reader.
    let reader = Reader::new(Rc::new(|_: &str| None));
    // The reader's own header and action bar are GTK chrome around the
    // document. ADR 0032 moves both *into* it, and this example is about what
    // the document does, so an empty header band above it would misrepresent
    // the result.
    reader.set_actions_visible(false);
    reader.header().widget().set_visible(false);

    let pane = reader.widget();
    // Fill the window, or the ground behind the pane shows as a white band
    // top and bottom and the shot misrepresents how this looks in the app.
    pane.set_vexpand(true);
    pane.set_hexpand(true);
    window.set_child(Some(&pane));
    window.present();
    pump();

    let before = resident_kib();
    let started = Instant::now();
    reader.render_thread(&messages);
    let handed = started.elapsed();
    // Let WebKit actually parse and paint it before anything is measured.
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(3) {
        pump();
        std::thread::sleep(Duration::from_millis(10));
    }

    println!("messages          {count}");
    println!("web processes     {}", web_processes());
    println!("document handed   {handed:.2?}");
    println!("document bytes    {}", reader.test_document().len());
    println!(
        "resident          {} MiB (+{} MiB over the empty view)",
        resident_kib() / 1024,
        resident_kib().saturating_sub(before) / 1024
    );

    // A picture of it, because "matches the canvas" is not checkable by
    // squinting at a running app -- and because a shot nobody looks at is a
    // shot that was not taken (#809).
    if let Ok(path) = std::env::var("POSTIO_SHOT") {
        match postio_gtk::capture::png(&window, std::path::Path::new(&path)) {
            Ok(_) => println!("wrote             {path}"),
            Err(error) => {
                eprintln!("NO IMAGE WAS WRITTEN: {error}");
                std::process::exit(1);
            }
        }
    }

    if quiet {
        // Measured and done: what this prints is the whole answer, and a
        // window nobody closes would hold a gate run open.
        return;
    }

    // Otherwise leave it up to be looked at.
    let main = glib::MainLoop::new(None, false);
    window.connect_close_request({
        let main = main.clone();
        move |_| {
            main.quit();
            glib::Propagation::Proceed
        }
    });
    main.run();
}
