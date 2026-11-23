use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::message::MessageBody;
use postio_widgets::reader::{Absent, Reader, RemoteImageAllowList};

fn reader_in_a_window() -> Option<(gtk::Window, Reader, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    crate::support_reader::prepare(&display);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        dir.path().join("allow.json"),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    Some((window, reader, dir))
}

fn lay_out() {
    let until = std::time::Instant::now()
        + postio_test_support::scaled(std::time::Duration::from_millis(250));
    while std::time::Instant::now() < until {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn body_top(reader: &Reader) -> f32 {
    reader
        .view()
        .compute_bounds(&reader.widget())
        .expect("the body is laid out inside the reader")
        .y()
}

fn header(reader: &Reader) {
    reader.set_message_header(
        &[EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        &[EmailAddress::new(Some("Grace Hopper"), "grace@example.com")],
        &[],
        Some("Figures"),
        Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
    );
}

fn plain(text: &str) -> MessageBody {
    MessageBody {
        text: Some(text.to_owned()),
        html: None,
    }
}

pub fn the_body_starts_at_the_same_place_whatever_the_notices() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    let mut tops = Vec::new();

    header(&reader);
    reader.render(&plain("a plain message"), Some("ada@example.com"));
    lay_out();
    tops.push(("no notice", body_top(&reader)));

    header(&reader);
    reader.render(&plain("a list message"), Some("ada@example.com"));
    reader.set_unsubscribe(Some("list.example.com"));
    lay_out();
    tops.push(("one notice", body_top(&reader)));

    header(&reader);
    reader.render(
        &MessageBody {
            text: None,
            html: Some(
                "<p>Figures attached.</p><img src=\"https://images.invalid/chart.png\">".to_owned(),
            ),
        },
        Some("ada@example.com"),
    );
    reader.set_encoding_problems(true);
    reader.set_unsubscribe(Some("list.example.com"));
    lay_out();
    assert!(
        reader.banner_visible() || reader.shows_encoding_problems(),
        "the third message raised no notice at all, so this proves nothing"
    );
    tops.push(("three notices", body_top(&reader)));

    header(&reader);
    reader.show_absent(Absent::Partial);
    lay_out();
    tops.push(("the waiting plate", body_top(&reader)));

    let first = tops[0].1;
    assert!(
        tops.iter().all(|(_, top)| (*top - first).abs() < 0.5),
        "the body's first line moved between messages: {tops:?}"
    );

    window.close();
}

pub fn a_sender_warning_is_one_line_and_does_not_outlast_its_message() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    header(&reader);
    reader.render(&plain("an ordinary message"), Some("ada@example.com"));
    lay_out();
    assert_eq!(
        reader.sender_warning_label(),
        None,
        "ordinary mail is quiet"
    );
    let quiet = body_top(&reader);

    let said = "This message failed sender verification: it may not be from example.com.";
    reader.set_sender_warning(Some(said));
    lay_out();
    assert_eq!(reader.sender_warning_label().as_deref(), Some(said));
    // A busy machine can need more than one frame to size the new row.
    for _ in 0..20 {
        if body_top(&reader) > quiet {
            break;
        }
        lay_out();
    }
    assert!(
        body_top(&reader) > quiet,
        "the warning took no room above the body"
    );

    header(&reader);
    lay_out();
    assert_eq!(reader.sender_warning_label(), None);
    assert!((body_top(&reader) - quiet).abs() < 0.5);

    window.close();
}

fn with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if root.has_css_class(class) {
        found.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        found.extend(with_class(&current, class));
        child = current.next_sibling();
    }
    found
}

fn action_cap(reader: &Reader, notice: &str) -> Option<String> {
    let caps = with_class(&reader.widget(), &format!("{notice}-action-hint"));
    let cap = caps
        .first()
        .unwrap_or_else(|| panic!("{notice} has no action button"));
    cap.is_visible().then(|| {
        cap.downcast_ref::<gtk::Label>()
            .expect("a cap is a label")
            .label()
            .to_string()
    })
}

pub fn every_notice_action_carries_its_key() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    header(&reader);
    reader.render(&plain("a list message"), Some("ada@example.com"));
    reader.set_unsubscribe(Some("list.example.com"));
    lay_out();
    assert!(
        reader.unsubscribe_banner_visible(),
        "the list's notice is not on screen, so its cap proves nothing"
    );
    let caps = || {
        [
            (
                "Unsubscribe",
                action_cap(&reader, "postio-unsubscribe-banner"),
            ),
            ("Show images", action_cap(&reader, "postio-remote-banner")),
        ]
    };
    assert_eq!(
        caps(),
        [
            ("Unsubscribe", Some("M".to_owned())),
            ("Show images", Some("alt+i".to_owned())),
        ],
        "a notice's button without its key"
    );

    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("unsubscribe".to_owned(), "g u".to_owned());
    overrides
        .overrides_mut()
        .insert("show_images".to_owned(), "I".to_owned());
    reader.set_keymap(&postio_core::Keymap::resolve(&overrides));
    assert_eq!(
        caps(),
        [
            ("Unsubscribe", Some("g u".to_owned())),
            ("Show images", Some("I".to_owned())),
        ],
        "a rebind reaches the keyboard and not the notice"
    );

    window.close();
}
