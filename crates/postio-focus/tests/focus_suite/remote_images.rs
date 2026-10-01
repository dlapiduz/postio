//! HTML-only mail with remote images, opened in Focus (US2 scenario 4,
//! T072): drawn from its HTML, sanitised, its images blocked until the
//! sender is allowed, no script run, and no request made -- over the corpus
//! fixtures the reader's own no-request cases use.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
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
        support::keys(&window, &["j"]);
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
