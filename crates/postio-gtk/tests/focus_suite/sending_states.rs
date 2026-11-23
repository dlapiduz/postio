use postio_model::{Draft, DraftState, EmailAddress};
use postio_storage::repository::DraftRepository;

use crate::support::{self, Fixture};

pub(crate) const SUBJECT: &str = "Tide gate interlock";

pub(crate) async fn with_a_draft(
    state: DraftState,
) -> (
    Fixture,
    postio_gtk::window::FocusWindow,
    postio_model::DraftId,
) {
    let fixture = Fixture::empty().await;
    {
        let connection = fixture.database.connect().await.expect("a connection");
        postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
    }
    fixture.file_from_ada("Budget", "Numbers.", 5).await;
    let draft = {
        let connection = fixture.database.connect().await.expect("a connection");
        let drafts = DraftRepository::new(&connection);
        let mut draft = Draft::new(fixture.account.id);
        draft.subject = SUBJECT.to_owned();
        draft.to = vec![EmailAddress::new(None::<String>, "quinn@example.net")];
        draft.body.text = Some("It may have gone.".to_owned());
        let id = drafts.save(&mut draft).await.expect("save the draft");
        match state {
            DraftState::Queued => {
                drafts
                    .queue_send(&mut draft, chrono::Utc::now())
                    .await
                    .expect("queue the send");
            }
            DraftState::Editing => {}
            other => drafts.set_state(id, other).await.expect("the state"),
        }
        id
    };
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window) == ["Budget"]).await,
        "the inbox never reached the screen"
    );
    assert!(
        crate::settle_until(async || window.composer().is_some()).await,
        "Focus mounted no composer"
    );
    (fixture, window, draft)
}

async fn state_of(fixture: &Fixture, draft: postio_model::DraftId) -> Option<DraftState> {
    let connection = fixture.database.connect().await.expect("a connection");
    DraftRepository::new(&connection)
        .get(draft)
        .await
        .expect("read the draft")
        .map(|draft| draft.state)
}

fn row_texts(window: &postio_gtk::window::FocusWindow, subject: &str) -> Option<Vec<String>> {
    window
        .pane()?
        .rows_on_screen()
        .into_iter()
        .map(|row| row.drawn().texts)
        .find(|texts| texts.iter().any(|text| text == subject))
}

pub(crate) async fn go_to(window: &postio_gtk::window::FocusWindow, name: &str) {
    support::keys(window, &["g", "l"]);
    let places = window.places().expect("g l opened the folders popover");
    assert!(
        crate::settle_until(async || places.names().contains(&name.to_owned())).await,
        "the folders popover does not list {name}: {:?}",
        places.names()
    );
    places.set_filter(name);
    support::click_row_saying(window, window, name);
    assert!(
        crate::settle_until(async || window.place_name() == name).await,
        "the list did not go to {name}: {}",
        window.place_name()
    );
}

pub(crate) async fn open_the_draft(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || support::subjects(window) == [SUBJECT]).await,
        "the draft is not listed: {:?}",
        support::subjects(window)
    );
    support::deliver(window, "j");
    support::deliver(window, "Return");
}

pub(crate) async fn reading(
    window: &postio_gtk::window::FocusWindow,
) -> std::rc::Rc<postio_gtk::open::OpenMessage> {
    assert!(
        crate::settle_until(async || {
            window
                .reading()
                .is_some_and(|reading| reading.is_open() && reading.title() == SUBJECT)
        })
        .await,
        "Return on the draft opened no message (composer open: {})",
        window.compose_dialog().is_some()
    );
    window.reading().expect("the open message")
}

fn verbs(reading: &postio_gtk::open::OpenMessage) -> Vec<String> {
    let view = reading.view();
    [
        "Reply",
        "Archive",
        "Cancel send",
        "Retry send",
        "Mark as sent",
        "Edit",
    ]
    .into_iter()
    .filter(|verb| {
        support::descendants(&view).into_iter().any(|widget| {
            widget.is::<gtk::Button>()
                && gtk::prelude::WidgetExt::is_mapped(&widget)
                && support::texts(&widget).iter().any(|text| text == verb)
        })
    })
    .map(str::to_owned)
    .collect()
}

use gtk::prelude::*;

pub fn an_unconfirmed_send_says_so_and_mark_as_sent_settles_it() {
    crate::gtk_case(async {
        let (fixture, window, draft) = with_a_draft(DraftState::Unconfirmed).await;
        support::keys(&window, &["g", "d"]);
        assert!(
            crate::settle_until(async || {
                row_texts(&window, SUBJECT)
                    .is_some_and(|texts| texts.iter().any(|text| text == "Not confirmed"))
            })
            .await,
            "the Drafts row does not say Not confirmed: {:?}",
            row_texts(&window, SUBJECT)
        );

        open_the_draft(&window).await;
        let reading = reading(&window).await;
        assert!(
            crate::settle_until(async || verbs(&reading) == ["Retry send", "Mark as sent", "Edit"])
                .await,
            "an unconfirmed send offers {:?}",
            verbs(&reading)
        );
        assert!(
            window.compose_dialog().is_none(),
            "looking at a send nobody confirmed opened the composer"
        );

        let mark = support::button_labelled(&reading.view(), "Mark as sent");
        support::click(&window, &mark, 1);
        assert!(
            crate::settle_until(async || state_of(&fixture, draft).await == Some(DraftState::Sent))
                .await,
            "Mark as sent did not record the send: {:?}",
            state_of(&fixture, draft).await
        );
        assert!(
            crate::settle_until(async || row_texts(&window, SUBJECT).is_none()).await,
            "the settled send is still in Drafts: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "the open message stayed on a draft that left the list"
        );
    });
}

pub fn received_mail_keeps_its_verbs_and_the_send_keys_say_why_not() {
    crate::gtk_case(async {
        let (fixture, window, draft) = with_a_draft(DraftState::Unconfirmed).await;
        support::deliver(&window, "j");
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || {
                window
                    .reading()
                    .is_some_and(|reading| reading.is_open() && reading.title() == "Budget")
            })
            .await,
            "Return on the inbox row opened nothing"
        );
        let reading = window.reading().expect("the open message");
        assert!(
            crate::settle_until(async || verbs(&reading) == ["Reply", "Archive"]).await,
            "received mail offers {:?}",
            verbs(&reading)
        );
        support::deliver_with(
            &window,
            "m",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(
            crate::settle_until(async || {
                window.toast_showing().as_deref() == Some("That message is not one being sent")
            })
            .await,
            "Mark as sent over received mail said {:?}",
            window.toast_showing()
        );
        assert_eq!(
            state_of(&fixture, draft).await,
            Some(DraftState::Unconfirmed),
            "Mark as sent over received mail settled a draft it was not aimed at"
        );
    });
}
