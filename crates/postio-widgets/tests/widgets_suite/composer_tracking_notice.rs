use crate::support_compose::Window;
use crate::support_compose::composer;
use crate::support_compose::find;
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_model::{Account, AccountId, EmailAddress, Message, MessageBody};

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

fn an_account() -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "grace@example.com"),
    );
    account.id = AccountId::new(1);
    account
}

fn html_only_message(from_domain: &str, link_host: &str) -> Message {
    let mut message = Message::new(
        AccountId::new(1),
        postio_model::ids::MailboxId::new(1),
        chrono::Utc::now(),
    );
    message.id = postio_model::ids::MessageId::new(7);
    message.from = vec![EmailAddress::new(
        Some("Cooperage Supply"),
        format!("orders@{from_domain}"),
    )];
    message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
    message.subject = Some("Your order".to_owned());
    message.body = MessageBody {
        text: None,
        html: Some(format!(
            "<p><a href=\"https://{link_host}/r?c=aa71\">Shop now</a></p>"
        )),
    };
    message
}

fn notice(composer: &postio_widgets::composer::Composer) -> gtk::Label {
    find(
        composer.clone().upcast_ref::<gtk::Widget>(),
        "postio-compose-tracking-notice",
    )
    .expect("the composer builds the notice label unconditionally")
    .downcast()
    .expect("the notice is a label")
}

pub fn replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not() {
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    let notice = notice(&composer);
    assert!(
        !notice.is_visible(),
        "nothing has been replied to yet, so there is nothing to say"
    );

    composer.connect_reply_source({
        let source = html_only_message("shop.example.org", "click.tracker.example.org");
        move |answer| answer(Some((source.clone(), an_account())))
    });
    press(&window, "r");
    assert!(composer.is_open());
    assert!(notice.is_visible());
    assert!(
        notice.text().contains("click.tracker.example.org"),
        "{}",
        notice.text()
    );
    composer.discard();
    settle();
    assert!(
        !notice.is_visible(),
        "discarding the reply must not leave a stale notice for the next composition"
    );

    composer.connect_reply_source({
        let source = html_only_message("shop.example.org", "shop.example.org");
        move |answer| answer(Some((source.clone(), an_account())))
    });
    press(&window, "r");
    assert!(composer.is_open());
    assert!(
        !notice.is_visible(),
        "a link to the sender's own domain is not a foreign tracker"
    );
}
