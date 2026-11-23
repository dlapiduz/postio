use gtk::gdk;
use postio_ui::source::Tab;

use crate::support::{self, Fixture};

const RAW: &[u8] = b"From: Ada Moreno <ada@example.com>\r\n\
To: you@example.com\r\n\
Subject: Budget\r\n\
X-Postio-Fixture: raw source\r\n\
\r\n\
The numbers are attached, see https://example.com/budget.\r\n";

fn on(window: &postio_gtk::window::FocusWindow, tab: Tab) -> bool {
    window.source().is_some_and(|source| source.tab() == tab)
}

pub fn v_opens_the_source_on_verification_and_the_digits_pick_its_tabs() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::keys(&window, &["j", "V"]);
        assert!(
            crate::settle_until(async || on(&window, Tab::Verification)).await,
            "V on the row did not open the Source surface on Verification"
        );
        let source = window.source().expect("open");
        assert!(
            source
                .text_of(Tab::Verification)
                .contains("These checks are about the sending domain"),
            "{}",
            source.text_of(Tab::Verification)
        );

        support::keys(&window, &["2"]);
        assert!(crate::settle_until(async || on(&window, Tab::Headers)).await);
        assert!(
            source.text_of(Tab::Headers).contains("Subject: Budget"),
            "{}",
            source.text_of(Tab::Headers)
        );
        support::keys(&window, &["3"]);
        assert!(crate::settle_until(async || on(&window, Tab::Raw)).await);
        let raw = source.text_of(Tab::Raw);
        assert!(raw.contains("X-Postio-Fixture: raw source"), "{raw}");
        assert!(
            raw.contains("https://example.com/budget"),
            "a URL is characters in the Raw tab"
        );
        support::keys(&window, &["1"]);
        assert!(crate::settle_until(async || on(&window, Tab::Verification)).await);

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.source().is_none()).await,
            "Escape did not close the source"
        );
        assert_eq!(window.observe().cursor.index, Some(0), "the same row");
    });
}

pub fn v_in_the_open_message_stacks_the_source_and_escape_takes_one_rung() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|reading| reading.is_open()))
                .await
        );
        support::keys(&window, &["V"]);
        assert!(
            crate::settle_until(async || on(&window, Tab::Verification)).await,
            "V in the open message did not open the Source surface"
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(crate::settle_until(async || window.source().is_none()).await);
        assert!(
            window.reading().is_some_and(|reading| reading.is_open()),
            "the first Escape closes only the source"
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !window
                .reading()
                .is_some_and(|reading| reading.is_open()))
            .await,
            "the second Escape closes the message"
        );
    });
}

pub fn save_as_eml_writes_the_bytes_the_store_holds() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        let (window, client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        let out = tempfile::tempdir().expect("a directory");
        let to = out.path().join("budget.eml");
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        window.set_file_picker({
            let (asked, to) = (std::rc::Rc::clone(&asked), to.clone());
            move |pick, answer| {
                asked.borrow_mut().push(pick);
                answer(Some(to.clone()));
            }
        });
        support::keys(&window, &["j", "V"]);
        assert!(crate::settle_until(async || on(&window, Tab::Verification)).await);
        support::press(&window, "s", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || to.exists()).await,
            "mod+s on the Source surface saved nothing"
        );
        assert_eq!(
            *asked.borrow(),
            [postio_gtk::chooser::SavePick::Message {
                suggested: "Budget.eml".to_owned()
            }]
        );
        let stored = client.raw_source(message).await.expect("the raw source");
        assert_eq!(
            std::fs::read(&to).expect("the saved file"),
            stored,
            "the saved file is the stored bytes"
        );
    });
}

pub fn signed_mark() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (message, _) = fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.write_raw(message, RAW).await;
        fixture.write_verdict(message, "good").await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window
                .reading()
                .is_some_and(|reading| reading.mark_shown().as_deref() == Some("Signed")))
            .await,
            "a good signature over the whole message says Signed"
        );
        window.reading().expect("open").press_mark();
        assert!(
            crate::settle_until(async || on(&window, Tab::Verification)).await,
            "the mark opens Verification"
        );
    });
}
