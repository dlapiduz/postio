//! Issue #12: where the signature sits on a rich reply, and who decides.
//!
//! The placement lives in `[compose]` and reaches the composer through
//! `config::install_at`; what this asserts is the half a config test cannot —
//! that the setting picks the *draft kind's* placement and that the signature
//! actually moves, in a composer holding a real quoted document.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use crate::settle;
use gtk::gdk;
use postio_body::Placement;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::Signature;
use postio_model::{Account, AccountId, Draft, DraftKind, EmailAddress, Identity, MessageBody};

/// An account whose one identity signs.
fn account() -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "lena@example.org"),
    );
    account.id = AccountId::new(1);
    let mut identity = Identity::new(
        account.id,
        EmailAddress::new(Some("Lena"), "lena@example.org"),
    );
    identity.id = postio_model::IdentityId::new(1);
    identity.is_default = true;
    identity.signature = Some(Signature {
        id: Default::default(),
        name: String::new(),
        text: "Lena Tomlin".to_owned(),
        html: Some("<p><strong>Lena Tomlin</strong></p>".to_owned()),
    });
    account.identities = vec![identity];
    account
}

/// A reply whose body is genuinely rich: a written line above a real quote.
fn rich_reply() -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.kind = DraftKind::Reply;
    draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
    draft.subject = "Re: the lamp".to_owned();
    draft.body = MessageBody {
        text: Some("Looking now.\n\n> Small diff.".to_owned()),
        html: Some("<p>Looking now.</p><blockquote><p>Small diff.</p></blockquote>".to_owned()),
    };
    draft
}

pub fn the_configured_placement_decides_which_side_of_the_quote_signs() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.set_identities(account().identities.clone());

    // ── above the quote: what a top-posting reply looks like ─────────────
    composer.set_signature_placement(Placement::AboveQuote, Placement::AboveQuote);
    composer.open(rich_reply());
    settle();

    let document = composer.document();
    let text = document.to_text();
    let signature_at = text.find("Lena Tomlin").expect("the signature");
    let quote_at = text.find("> Small diff.").expect("the quote");
    assert!(
        signature_at < quote_at,
        "a reply configured to sign above the quote signed below it:\n{text}"
    );
    // The rich variant, not its flattened text: the body is a document and
    // the signature is part of it.
    assert!(
        document.to_html().contains("<strong>Lena Tomlin</strong>"),
        "{}",
        document.to_html()
    );
    // Signed once.
    assert_eq!(text.matches("-- ").count(), 1, "{text}");

    // ── and below it, on the same draft, without stacking ────────────────
    composer.discard();
    settle();
    composer.set_signature_placement(Placement::BelowQuote, Placement::BelowQuote);
    composer.open(rich_reply());
    settle();

    let text = composer.document().to_text();
    let signature_at = text.find("Lena Tomlin").expect("the signature");
    let quote_at = text.find("> Small diff.").expect("the quote");
    assert!(
        signature_at > quote_at,
        "a reply configured to sign below the quote signed above it:\n{text}"
    );
    assert_eq!(text.matches("-- ").count(), 1, "{text}");

    // ── a named signature, chosen without changing the identity ──────────
    composer.discard();
    settle();
    composer.set_signatures(vec![
        Signature::new("Short", "— L"),
        Signature::new("Long", "Lena Tomlin\nPostio"),
    ]);
    composer.open(rich_reply());
    settle();

    let from = composer.identity().map(|identity| identity.id);
    composer.test_choose_signature(1);
    settle();
    let text = composer.document().to_text();
    assert!(
        text.contains("— L"),
        "the chosen signature is the one used:\n{text}"
    );
    assert!(!text.contains("Lena Tomlin"), "{text}");
    assert_eq!(text.matches("-- ").count(), 1, "signed once: {text}");
    assert_eq!(
        composer.identity().map(|identity| identity.id),
        from,
        "choosing a signature must not change who the message is from"
    );

    // Switching to another replaces rather than appends.
    composer.test_choose_signature(2);
    settle();
    let text = composer.document().to_text();
    assert!(text.contains("Lena Tomlin"), "{text}");
    assert!(
        !text.contains("— L"),
        "the old signature stayed behind:\n{text}"
    );
    assert_eq!(text.matches("-- ").count(), 1, "{text}");

    // And back to the identity's own.
    composer.test_choose_signature(0);
    settle();
    let text = composer.document().to_text();
    assert!(text.contains("Lena Tomlin"), "{text}");
    assert_eq!(text.matches("-- ").count(), 1, "{text}");
}
