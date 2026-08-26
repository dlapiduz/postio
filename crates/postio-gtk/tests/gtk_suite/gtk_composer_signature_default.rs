//! A brand-new draft signs with whatever `postio_model::signature_default`
//! resolves, before the identity's own (#394).
//!
//! The precedence itself is pure logic, unit-tested in `postio-model`. What
//! needs a display is the composer's seam: that
//! `connect_signature_default`'s answer actually lands on the picker when
//! `c` starts a new draft, that it is read fresh every time rather than
//! once at mount, and that a `None` answer resets a picker a previous
//! compose left pointed at something else.

use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_model::ids::IdentityId;
use postio_model::{Account, EmailAddress, Identity, Signature, SignatureId};

/// An account with an identity that signs with its own line, plus two named
/// signatures the picker can offer instead.
fn account() -> Account {
    let mut account = Account::new(
        "Mo Reyes",
        EmailAddress::new(Some("Mo Reyes"), "mo@example.com"),
    );
    let mut identity = Identity::new(account.id, EmailAddress::new(Some("Mo"), "mo@example.com"));
    identity.id = IdentityId::new(1);
    identity.is_default = true;
    identity.signature = Some(Signature {
        id: Default::default(),
        name: String::new(),
        text: "Mo".to_owned(),
        html: None,
    });
    account.identities = vec![identity];
    account.signatures = vec![
        {
            let mut sig = Signature::new("Support", "Mo — Support");
            sig.id = SignatureId::new(1);
            sig
        },
        {
            let mut sig = Signature::new("Sales", "Mo — Sales");
            sig.id = SignatureId::new(2);
            sig
        },
    ];
    account
}

fn body(composer: &composer::Composer) -> String {
    composer.draft().body.text.unwrap_or_default()
}

pub fn a_resolved_signature_wins_over_the_identity_s_own() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let window = Window::default();
    let composer = composer::install(&window);
    let account = account();
    composer.set_identities(account.identities.clone());
    composer.set_signatures(account.signatures.clone());

    composer.connect_signature_default(|| Some(SignatureId::new(2)));
    assert!(
        gtk::prelude::WidgetExt::activate_action(&window, "win.compose", None).is_ok(),
        "win.compose should be reachable"
    );

    assert_eq!(
        body(&composer),
        "\n\n-- \nMo — Sales",
        "the resolved signature should have signed the new draft, not the identity's own"
    );
}

pub fn a_resolved_signature_the_account_does_not_have_falls_back_to_the_identity() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let window = Window::default();
    let composer = composer::install(&window);
    let account = account();
    composer.set_identities(account.identities.clone());
    composer.set_signatures(account.signatures.clone());

    // A stale or foreign id -- e.g. a deleted signature the mailbox row
    // still names -- must not panic or leave the picker on whatever it last
    // held; it falls all the way back to the identity's own.
    composer.connect_signature_default(|| Some(SignatureId::new(999)));
    assert!(gtk::prelude::WidgetExt::activate_action(&window, "win.compose", None).is_ok());

    assert_eq!(body(&composer), "\n\n-- \nMo");
}

pub fn no_resolution_resets_a_picker_a_previous_compose_left_pointed_elsewhere() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let window = Window::default();
    let composer = composer::install(&window);
    let account = account();
    composer.set_identities(account.identities.clone());
    composer.set_signatures(account.signatures.clone());

    // First compose: a mailbox override picks "Support".
    composer.connect_signature_default(|| Some(SignatureId::new(1)));
    assert!(gtk::prelude::WidgetExt::activate_action(&window, "win.compose", None).is_ok());
    assert_eq!(body(&composer), "\n\n-- \nMo — Support");
    composer.discard();

    // Second compose, a different mailbox with no opinion of its own and no
    // account default: resolves to nothing, and must not inherit "Support"
    // from the picker's last position.
    composer.connect_signature_default(|| None);
    assert!(gtk::prelude::WidgetExt::activate_action(&window, "win.compose", None).is_ok());
    assert_eq!(
        body(&composer),
        "\n\n-- \nMo",
        "a compose with no resolved signature kept the previous compose's pick"
    );
}
