//! The same thread, both ways, so ADR 0032's open question has a number (#1348).
//!
//! ADR 0032 asks it and cannot answer it:
//!
//! > Does WebKit's own memory for one large document beat N small processes?
//! > Not measured. It is the obvious rebuttal to "one view is cheaper" and
//! > nothing here has tested it.
//!
//! `thread_document.rs` measures the proposed arrangement. This measures both,
//! over the same fixture, with the same instrument, so the two columns can be
//! put beside each other honestly.
//!
//! ```sh
//! cargo run -p postio-app --example pane_comparison -- stacked 10
//! cargo run -p postio-app --example pane_comparison -- document 10
//! ```
//!
//! **One arrangement per process, deliberately.** Web processes outlive the
//! `Reader` that started them by however long it takes them to notice, so
//! measuring both in one run reports the first arrangement's processes as part
//! of the second's. Two invocations cost a few seconds and cannot lie that way.
//!
//! Nothing here touches the network or reads anybody's mail: the thread is
//! written in this file, at reserved domains.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::reader::view::{Reader, ThreadMessage};
use postio_gtk::reader::{BlobSource, RemoteImageAllowList};
use postio_gtk::{app, fonts, style};
use postio_model::message::MessageBody;

/// No inline images; `cid:` resolution is not what is being measured.
struct NoBlobs;
impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

/// A message with enough prose that a document is not trivially small.
fn body(index: usize) -> MessageBody {
    let people = ["Ada Lovelace", "Grace Hopper", "Hedy Lamarr"];
    let who = people[index % people.len()];
    MessageBody {
        text: None,
        html: Some(format!(
            "<p>Message {index} from {who}. The scope has three parts and I will \
             go through each so there are no surprises on the day.</p>\
             <p><b>Diagnostics.</b> We place two continuous monitors for 48 hours \
             — one in the lowest livable level, one a floor above — and pull a \
             soil-gas reading at the slab.</p>\
             <blockquote><p>Quoted from the message before it, so the folding \
             path has something to fold.</p></blockquote>"
        )),
    }
}

fn messages(count: usize) -> Vec<ThreadMessage> {
    let people = [
        ("Ada Lovelace", "ada@example.com"),
        ("Grace Hopper", "grace@example.org"),
        ("Hedy Lamarr", "hedy@example.net"),
    ];
    (0..count)
        .map(|index| {
            let (sender, address) = people[index % people.len()];
            ThreadMessage {
                scope: index.to_string(),
                sender: sender.to_owned(),
                address: address.to_owned(),
                when: "24 Aug 09:41".to_owned(),
                recipients: String::new(),
                cc: String::new(),
                preview: format!("Message {index}"),
                expanded: true,
                latest: index + 1 == count,
                draft: false,
                mine: false,
                body: body(index),
            }
        })
        .collect()
}

/// This process's resident set, in KiB.
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
/// By the parent chain, not `pgrep -P`: WebKit puts the web process under a
/// `bwrap` sandbox, so it is a grandchild and `-P` reports zero — which reads
/// exactly like the answer a one-document experiment wants and is a lie. And
/// `-x` against the truncated `comm`, because Linux cuts it to fifteen
/// characters.
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

/// The proportional set size of every web process this run owns, in KiB.
///
/// Without this the memory comparison is not merely incomplete, it is
/// *backwards in magnitude*: `/proc/self/status` is the UI process alone, and
/// the arrangement being measured puts its cost in fifty sandboxed children
/// that do not appear there at all. Reporting only the UI process would have
/// understated the stacked arrangement by most of what it actually costs.
fn web_process_kib() -> u64 {
    let mine = std::process::id() as i32;
    std::process::Command::new("pgrep")
        .args(["-x", "WebKitWebProces"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<i32>().ok())
                .filter(|pid| descends_from(*pid, mine))
                .filter_map(|pid| {
                    // **Pss, not VmRSS.** Fifty web processes map the same
                    // WebKit libraries, and RSS counts those pages in full in
                    // every one of them -- which inflates the stacked
                    // arrangement by most of its apparent cost and would turn
                    // a real finding into an indefensible one. Pss divides
                    // each shared page by the number of processes mapping it,
                    // which is the only figure that can be summed across
                    // processes and compared against one.
                    let rollup =
                        std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
                    rollup
                        .lines()
                        .find(|line| line.starts_with("Pss:"))?
                        .split_whitespace()
                        .nth(1)?
                        .parse::<u64>()
                        .ok()
                })
                .sum()
        })
        .unwrap_or(0)
}

fn descends_from(mut pid: i32, ancestor: i32) -> bool {
    for _ in 0..10 {
        if pid == ancestor {
            return true;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(after) = stat.rsplit(especially_the_close_paren()).next() else {
            return false;
        };
        let Some(parent) = after.split_whitespace().nth(1).and_then(|p| p.parse().ok()) else {
            return false;
        };
        pid = parent;
    }
    false
}

/// `/proc/<pid>/stat`'s second field is the command in parentheses and may
/// contain spaces, so the fields after it are found from the last `)`.
fn especially_the_close_paren() -> char {
    ')'
}

fn pump() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle(how_long: Duration) {
    let started = Instant::now();
    while started.elapsed() < how_long {
        pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let arrangement = args.next().unwrap_or_else(|| "document".to_owned());
    let count: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(10);

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("no display: run under scripts/test-headless.sh or a session");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("fonts");
    style::install(&display);
    app::install_icons(&display);

    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_child(Some(&column));
    window.present();
    pump();

    let thread = messages(count);
    let documents_before = postio_ui::test_support::documents_built();
    let bytes_before = postio_ui::test_support::document_bytes();
    let renders_before = postio_ui::test_support::renders_issued();
    let resident_before = resident_kib();

    // Held for the whole measurement: dropping a reader releases its web
    // process, and the question is what the arrangement costs while a person
    // is looking at it.
    let mut readers: Vec<Reader> = Vec::new();
    let started = Instant::now();

    match arrangement.as_str() {
        // What the pane does today: one `Reader` — one `WebView`, one web
        // process — per message on screen.
        "stacked" => {
            for message in &thread {
                let reader = Reader::with_allowlist(
                    Rc::new(NoBlobs),
                    RemoteImageAllowList::default(),
                    std::env::temp_dir().join("pane-comparison-allowlist.toml"),
                );
                let pane = reader.widget();
                pane.set_vexpand(true);
                column.append(&pane);
                reader.render(&message.body, Some(&message.sender));
                readers.push(reader);
            }
        }
        // ADR 0032: one reader for the whole thread.
        "document" => {
            let reader = Reader::with_allowlist(
                Rc::new(NoBlobs),
                RemoteImageAllowList::default(),
                std::env::temp_dir().join("pane-comparison-allowlist.toml"),
            );
            let pane = reader.widget();
            pane.set_vexpand(true);
            column.append(&pane);
            reader.render_thread(&thread);
            readers.push(reader);
        }
        other => {
            eprintln!("unknown arrangement {other:?}: expected `stacked` or `document`");
            return;
        }
    }

    let handed = started.elapsed();
    // Let WebKit parse and paint before anything is measured.
    settle(Duration::from_secs(3));

    println!("arrangement       {arrangement}");
    println!("messages          {count}");
    println!("web processes     {}", web_processes());
    println!("handover          {handed:.2?}");
    println!(
        "documents built   {}",
        postio_ui::test_support::documents_built() - documents_before
    );
    println!(
        "renders issued    {}",
        postio_ui::test_support::renders_issued() - renders_before
    );
    println!(
        "document bytes    {}",
        postio_ui::test_support::document_bytes() - bytes_before
    );
    let ui = resident_kib();
    let web = web_process_kib();
    println!(
        "resident ui       {} MiB (+{} MiB)",
        ui / 1024,
        ui.saturating_sub(resident_before) / 1024
    );
    println!("web pss           {} MiB", web / 1024);
    println!(
        "total             {} MiB (ui rss + web pss)",
        (ui + web) / 1024
    );
}
