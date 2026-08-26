//! Identities in the composer, on a real display.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_cheatsheet.rs`.
//!
//! Which identity a reply belongs to, and how a signature is inserted without
//! being duplicated, are pure rules unit-tested in `postio-model`
//! (`Account::identity_for`, `signature::apply`). What needs a display is the
//! composer around them: that the `From` row really lands on the address the
//! mail was sent to, that switching it swaps the signature in the live body
//! instead of stacking a second one, and that the override belongs to the
//! draft rather than to the composer — closing and reopening still sends from
//! the address the user chose.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, IdentityId};
use postio_model::{Account, Draft, DraftKind, EmailAddress, Identity, Signature};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// An account that can send from two addresses, each signing differently.
fn account() -> Account {
    let mut account = Account::new(
        "Lena Tomlin",
        EmailAddress::new(Some("Lena Tomlin"), "lena@example.com"),
    );
    let identity = |id, address: &str, signature: &str, default| {
        let mut identity =
            Identity::new(account.id, EmailAddress::new(Some("Lena Tomlin"), address));
        identity.id = IdentityId::new(id);
        identity.is_default = default;
        identity.signature = Some(Signature {
            id: Default::default(),
            name: String::new(),
            text: signature.to_owned(),
            html: None,
        });
        identity
    };
    account.identities = vec![
        identity(1, "lena@example.com", "Lena", true),
        identity(2, "lena@work.example.org", "Lena Tomlin · Postio", false),
    ];
    account
}

/// The body of the composer, as a screen would show it.
fn body(composer: &composer::Composer) -> String {
    composer.draft().body.text.unwrap_or_default()
}

#[test]
fn the_reply_comes_from_the_address_it_was_sent_to_and_signs_once() {
    let state_dir = std::env::temp_dir().join(format!("postio-identity-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let account = account();
    let composer = composer::install(&window);
    composer.set_identities(account.identities.clone());

    // ── New mail comes from the account's default ────────────────────────
    press(&window, "c", gdk::ModifierType::empty());
    assert_eq!(
        composer.identity().map(|identity| identity.id),
        Some(IdentityId::new(1)),
        "an unprompted compose is from the default identity"
    );
    assert_eq!(
        body(&composer),
        "\n\n-- \nLena",
        "and it opens already signed, above the signature"
    );
    composer.discard();
    settle();

    // ── A reply comes from the address the mail was addressed to ─────────
    //
    // This is the seam `postio-p8q` will build the reply draft through: the
    // identity is chosen from the recipients of the message being answered.
    let addressed_to = vec![
        EmailAddress::new(None::<String>, "lena@work.example.org"),
        EmailAddress::new(None::<String>, "someone@example.net"),
    ];
    let chosen = account
        .identity_for(&addressed_to)
        .expect("the account has identities");
    assert_eq!(chosen.id, IdentityId::new(2), "matched on the work address");

    let mut reply = Draft::new(AccountId::new(1));
    reply.kind = DraftKind::Reply;
    reply.to = vec![EmailAddress::new(
        Some("Diogo Ferreira"),
        "diogo@example.org",
    )];
    reply.subject = "Re: mbox importer review".to_owned();
    reply.body.text = Some("Looking now.\n\n> Small diff.\n".to_owned());
    reply.use_identity(chosen);

    composer.open(reply);
    settle();

    assert_eq!(
        composer.identity().map(|identity| identity.id),
        Some(IdentityId::new(2)),
        "the reply's own identity wins over the account default"
    );
    assert_eq!(
        body(&composer),
        "Looking now.\n\n> Small diff.\n\n-- \nLena Tomlin · Postio",
        "signed once, below the quote, where RFC 3676 says a signature goes"
    );
    assert_eq!(
        body(&composer).matches("-- \n").count(),
        1,
        "and not signed twice by opening it"
    );

    // ── Switching identity swaps the signature rather than adding one ────
    assert!(composer.select_identity(IdentityId::new(1)));
    settle();
    assert_eq!(
        body(&composer),
        "Looking now.\n\n> Small diff.\n\n-- \nLena",
        "the old signature went with the old identity"
    );

    // ── The override belongs to the draft ────────────────────────────────
    press(&window, "Escape", gdk::ModifierType::empty());
    assert!(!composer.is_open());
    press(&window, "c", gdk::ModifierType::empty());
    assert_eq!(
        composer.identity().map(|identity| identity.id),
        Some(IdentityId::new(1)),
        "reopening the kept draft sends from the address it was set to"
    );
    assert_eq!(
        composer.draft().identity_id,
        Some(IdentityId::new(1)),
        "and the draft carries it, for whoever sends it"
    );
    assert_eq!(
        body(&composer).matches("-- \n").count(),
        1,
        "reopening a draft does not re-sign it"
    );
}
