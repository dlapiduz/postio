use crate::support_compose::Window;
use crate::support_compose::composer;
use postio_model::ids::IdentityId;
use postio_model::{Account, EmailAddress, Identity, Signature, SignatureId};

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
    let window = Window::default();
    let composer = composer::install(&window);
    let account = account();
    composer.set_identities(account.identities.clone());
    composer.set_signatures(account.signatures.clone());

    composer.connect_signature_default(|answer| answer(Some(SignatureId::new(2))));
    assert!(
        gtk::prelude::WidgetExt::activate_action(window.gtk(), "win.compose", None).is_ok(),
        "win.compose should be reachable"
    );

    assert_eq!(
        body(&composer),
        "\n\n-- \nMo — Sales",
        "the resolved signature should have signed the new draft, not the identity's own"
    );
}
