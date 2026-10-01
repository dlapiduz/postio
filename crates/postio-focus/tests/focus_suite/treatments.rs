//! The open message's two treatments (specs/007-postio-focus T210-T213): a
//! newsletter that paints its page opens on paper, office mail in app
//! colours, the line above the body names which, `O` switches the message
//! on screen through the keys a person presses, and "Always for this
//! sender" is remembered for the next time it opens.

use std::cell::RefCell;
use std::rc::Rc;

use postio_body::treatment::Treatment;
use postio_model::test_corpus;

use crate::support::{self, Fixture};

fn html_of(name: &str) -> String {
    postio_model::mime::parse(test_corpus::load(name).bytes())
        .body
        .html
        .unwrap_or_else(|| panic!("{name} has an HTML part"))
}

/// An address no other case files mail from: what this case remembers for
/// it lives in the suite's shared state for the rest of the run.
const NEWS: &str = "news@treatments.example.com";

pub fn a_newsletter_opens_on_paper_and_o_switches_it_to_app_colours() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (news, _) = fixture
            .file(("Field Notes Weekly", NEWS), "Issue 48", "The light.", 5)
            .await;
        fixture
            .write_html_body(news, &html_of("html-newsletter-own-page"))
            .await;
        let (work, _) = fixture
            .file(
                ("Dana Whitfield", "facilities@treatments.example.com"),
                "Building access",
                "Hi everyone.",
                10,
            )
            .await;
        fixture
            .write_html_body(work, &html_of("html-work-black-text"))
            .await;
        let (plain, _) = fixture
            .file(("Ada", "ada@treatments.example.com"), "Lunch", "Noon?", 15)
            .await;
        fixture
            .write_body(plain, "Noon at the usual place?\n")
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );

        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("open");
        let reader = reading.reader();
        let seen: Rc<RefCell<Vec<Treatment>>> = Rc::default();
        reader.connect_treatment_changed({
            let seen = Rc::clone(&seen);
            move |treatment| seen.borrow_mut().push(treatment)
        });
        let line = reader.render_mode_line().expect("Focus draws the line");
        assert!(
            crate::settle_until(async || reader.treated().is_some()).await,
            "the newsletter was never drawn under a treatment"
        );
        assert_eq!(reader.treatment(), Treatment::Paper);
        assert!(line.is_shown(), "no line named the treatment");
        assert!(
            line.text()
                .starts_with("Original layout, on paper · this message sets its own background · Use app colours O"),
            "{}",
            line.text()
        );
        assert!(
            reader
                .test_document()
                .contains("data-postio-treatment=\"paper\""),
            "the body was not drawn on paper"
        );

        // `O`, pressed as a person presses it.
        support::keys(&window, &["O"]);
        assert!(
            crate::settle_until(async || reader.treatment() == Treatment::AppColours).await,
            "O did not switch the newsletter to app colours"
        );
        assert!(
            line.text()
                .starts_with("App colours · sender colours and fonts removed · Show original O"),
            "{}",
            line.text()
        );
        assert!(
            reader
                .test_document()
                .contains("data-postio-treatment=\"app\""),
            "the line changed and the body did not"
        );
        assert!(
            !reader.test_document().contains("f6f1e7"),
            "the sender's page colour reached app colours"
        );
        assert_eq!(seen.borrow().last(), Some(&Treatment::AppColours));
        assert!(
            line.offers_always(),
            "a choice against the rule can be remembered"
        );

        line.press_always();
        assert_eq!(
            reader.allowlist_snapshot().treatment_for(NEWS),
            Some(Treatment::AppColours),
            "Always for this sender was not stored with the sender's settings"
        );
        assert!(!line.offers_always());
        assert!(
            line.text().contains("always for this sender"),
            "{}",
            line.text()
        );

        // Office mail opens in app colours by the rule.
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "Building access").await,
            "j did not step to the office mail"
        );
        assert!(
            crate::settle_until(async || reader.test_document().contains("Facilities Coordinator"))
                .await,
            "the office mail never drew"
        );
        assert_eq!(reader.treatment(), Treatment::AppColours);
        assert!(line.text().starts_with("App colours"), "{}", line.text());
        assert!(!line.offers_always());

        // Plain text has no other treatment, and no line.
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || reading.title() == "Lunch").await,
            "j did not step to the plain message"
        );
        assert!(
            crate::settle_until(async || !line.is_shown()).await,
            "plain text was given a render-mode line"
        );
        assert_eq!(reader.treatment(), Treatment::AppColours);

        // Back to the newsletter: its sender's choice is remembered.
        support::keys(&window, &["k", "k"]);
        assert!(
            crate::settle_until(async || reading.title() == "Issue 48").await,
            "k did not step back to the newsletter"
        );
        assert!(
            crate::settle_until(
                async || line.is_shown() && line.text().contains("always for this sender")
            )
            .await,
            "the remembered choice was not applied: {}",
            line.text()
        );
        assert_eq!(reader.treatment(), Treatment::AppColours);
    });
}
