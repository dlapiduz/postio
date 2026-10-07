//! HTML-only mail with remote images, opened in Focus (US2 scenario 4,
//! T072): drawn from its HTML, sanitised, its images blocked until the
//! sender is allowed, no script run, and no request made -- over the corpus
//! fixtures the reader's own no-request cases use.

use std::cell::RefCell;
use std::rc::Rc;

use postio_model::test_corpus;

use crate::support::{self, Fixture};

fn html_of(name: &str) -> String {
    postio_model::mime::parse(test_corpus::load(name).bytes())
        .body
        .html
        .unwrap_or_else(|| panic!("{name} has an HTML part"))
}

pub fn remote_images_stay_blocked_scripts_go_and_nothing_is_asked_for() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (pixel, _) = fixture
            .file(("Shop", "offers@shop.example"), "New lamps", "Brass.", 5)
            .await;
        fixture
            .write_html_body(pixel, &html_of("html-tracking-pixel-remote-images"))
            .await;
        let (scripted, _) = fixture
            .file(
                ("Forms", "forms@forms.example"),
                "Your form",
                "Fill it.",
                10,
            )
            .await;
        fixture
            .write_html_body(scripted, &html_of("html-script-forms"))
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("open");
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        reading.reader().set_remote_fetch({
            let asked = Rc::clone(&asked);
            move |urls, _done| asked.borrow_mut().extend(urls)
        });
        assert!(
            crate::settle_until(async || reading.reader().banner_visible()).await,
            "the remote images were not held back behind the banner"
        );
        let document = reading.reader().test_document();
        // The images' hosts, not the domain: the fixture's click-tracking
        // link stays a link, which opens only when chosen.
        for host in [
            "pixel.tracker.example.org",
            "images.tracker.example.org",
            "cdn.tracker.example.org",
        ] {
            assert!(
                !document.contains(host),
                "an image from {host} survived into the document before the sender was allowed"
            );
        }
        // Longer than the dwell after which an allowed image would be asked
        // for: nothing is, because nothing is allowed.
        crate::settle_for(std::time::Duration::from_millis(1500)).await;
        assert!(
            asked.borrow().is_empty(),
            "something was asked for: {:?}",
            asked.borrow()
        );

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "Your form").await,
            "j did not step to the scripted message"
        );
        assert!(
            crate::settle_until(async || reading.body_text().contains("confirm your details"))
                .await,
            "the scripted message never drew"
        );
        let document = reading.reader().test_document();
        for gone in ["<script", "onload", "onerror", "beacon.example.com"] {
            assert!(!document.contains(gone), "{gone} survived the sanitiser");
        }
        assert!(
            asked.borrow().is_empty(),
            "something was asked for: {:?}",
            asked.borrow()
        );
    });
}

/// A window over `senders`' messages, newest first (so `j` lands on the
/// first listed), each with the tracking-pixel mail's HTML. The reader's
/// fetch is a recorder: what it holds is what the app asked the network
/// for.
async fn shop(
    senders: &[(&str, &str, &str)],
) -> (postio_gtk::window::FocusWindow, Rc<RefCell<Vec<String>>>) {
    let fixture = Fixture::empty().await;
    for (serial, (minutes, (name, address, subject))) in (5..).step_by(5).zip(senders).enumerate() {
        let (message, _) = fixture
            .file((name, address), subject, "Lamps.", minutes)
            .await;
        fixture
            // Each message's images are its own: a URL fetched for one is
            // not asked for again for the next.
            .write_html_body(
                message,
                &html_of("html-tracking-pixel-remote-images").replace(
                    "tracker.example.org",
                    &format!("m{serial}.tracker.example.org"),
                ),
            )
            .await;
    }
    let (window, _client) = fixture.open().await;
    support::keep(fixture);
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == senders.len()).await,
        "the inbox never reached the screen"
    );
    support::press(&window, "Return", gtk::gdk::ModifierType::empty());
    let reading = window.reading().expect("open");
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    reading.reader().set_remote_fetch({
        let asked = Rc::clone(&asked);
        move |urls, _done| asked.borrow_mut().extend(urls)
    });
    assert!(
        crate::settle_until(async || reading.reader().banner_visible()).await,
        "the remote images were not held back behind the banner"
    );
    (window, asked)
}

/// Show images, once: nothing is asked for until the button is pressed, then
/// the message's images are, and the next message from the sender is held
/// back again.
pub fn show_fetches_once_and_asks_for_nothing_before() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, asked) = shop(&[
            ("Shop", "once@shop.example", "New lamps"),
            ("Shop", "once@shop.example", "More lamps"),
        ])
        .await;
        let reading = window.reading().expect("open");
        crate::settle_for(std::time::Duration::from_millis(1500)).await;
        assert!(
            asked.borrow().is_empty(),
            "asked before the person did: {:?}",
            asked.borrow()
        );
        let show = support::button_labelled(&window, "Show images");
        support::click(&window, &show, 1);
        assert!(
            crate::settle_until(async || !asked.borrow().is_empty()).await,
            "Show images asked for nothing"
        );
        assert!(
            asked
                .borrow()
                .iter()
                .any(|url| url.contains("pixel.m0.tracker.example.org")),
            "the message's images: {:?}",
            asked.borrow()
        );
        let asked_for_first = asked.borrow().len();

        // Once means this message: the sender's next one is held back.
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "More lamps").await,
            "j did not step to the next message"
        );
        assert!(
            crate::settle_until(async || reading.reader().banner_visible()).await,
            "showing once allowed the sender"
        );
        crate::settle_for(std::time::Duration::from_millis(1500)).await;
        assert_eq!(
            asked.borrow().len(),
            asked_for_first,
            "the next message's images were asked for"
        );
    });
}

/// Always show from this sender (`i a`): the images are asked for, and the
/// sender's next message opens with them, no banner; another sender's does
/// not.
pub fn always_holds_for_the_senders_next_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, asked) = shop(&[
            ("Shop", "always@shop.example", "New lamps"),
            ("Shop", "always@shop.example", "More lamps"),
            ("Other", "stranger@other.example", "Lamps too"),
        ])
        .await;
        let reading = window.reading().expect("open");
        crate::settle_for(std::time::Duration::from_millis(1500)).await;
        assert!(asked.borrow().is_empty(), "asked before the person did");
        support::keys(&window, &["i", "a"]);
        assert!(
            crate::settle_until(async || !asked.borrow().is_empty()).await,
            "`i a` asked for nothing"
        );
        asked.borrow_mut().clear();
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "More lamps").await,
            "j did not step to the sender's next message"
        );
        assert!(
            crate::settle_until(async || !asked.borrow().is_empty()).await,
            "the sender's next message did not fetch its images"
        );
        assert!(
            !reading.reader().banner_visible(),
            "the banner came back for an allowed sender"
        );
        asked.borrow_mut().clear();
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "Lamps too").await,
            "j did not step to the stranger's message"
        );
        assert!(
            crate::settle_until(async || reading.reader().banner_visible()).await,
            "always allowed a sender who was never asked about"
        );
        crate::settle_for(std::time::Duration::from_millis(1500)).await;
        assert!(
            asked.borrow().is_empty(),
            "a stranger's images were asked for"
        );
    });
}
