//! `o`: open an attachment or a link (US2 scenario 9, T071). The message's
//! links and parts are offered, each link's full target shown before
//! anything opens, and nothing opens until one is chosen.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;

use crate::support::{self, Fixture};

pub fn o_offers_the_links_and_parts_and_opens_only_what_is_chosen() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let message = fixture.file_with_attachment("Invoice", "report.pdf").await;
        fixture
            .write_html_body(
                message,
                "<p>Here is <a href=\"https://example.com/invoice/42\">your invoice</a>.</p>",
            )
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let launched: Rc<RefCell<Vec<String>>> = Rc::default();
        window.set_launcher({
            let launched = Rc::clone(&launched);
            move |uri| launched.borrow_mut().push(uri.to_owned())
        });
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.body_text().contains("your invoice")).await,
            "the body never arrived"
        );

        support::keys(&window, &["o"]);
        assert!(
            crate::settle_until(async || !window.choices_shown().is_empty()).await,
            "o offered nothing"
        );
        let offered = window.choices_shown();
        assert!(
            offered.contains(&(
                "your invoice".to_owned(),
                "https://example.com/invoice/42".to_owned()
            )),
            "the link, with its full target: {offered:?}"
        );
        assert!(
            offered.iter().any(|(name, _)| name == "report.pdf"),
            "the attachment: {offered:?}"
        );
        assert!(
            launched.borrow().is_empty(),
            "something opened before a choice"
        );

        window.choose(0);
        assert_eq!(
            *launched.borrow(),
            ["https://example.com/invoice/42".to_owned()],
            "choosing the link opens it, and only it"
        );
        assert!(
            crate::settle_until(async || window.choices_shown().is_empty()).await,
            "the chooser stayed up after a choice"
        );
    });
}
