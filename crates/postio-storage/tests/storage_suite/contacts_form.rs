//! Saving the Edit form is one write that makes or changes a person whole
//! (specs/011-focus-contacts S8): names, kind, style, work, addresses, which
//! is primary, their labels and the groups, with the form as it was handed
//! back to put it right.

use postio_model::{
    AddressLabel, ContactForm, ContactKind, ContactState, EmailAddress, FormAddress, ShowAs,
};
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

fn row(address: &str, label: Option<AddressLabel>) -> FormAddress {
    FormAddress {
        address: address.to_owned(),
        label,
        ..FormAddress::blank()
    }
}

fn ada() -> ContactForm {
    ContactForm {
        kind: ContactKind::Person,
        given: "Ada".into(),
        family: "Moreno".into(),
        show_as: ShowAs::LastFirst,
        title: "Finance lead".into(),
        company: "Northwind Partners".into(),
        addresses: vec![
            row("ada@example.org", Some(AddressLabel::Work)),
            row("ada.moreno@example.com", Some(AddressLabel::Personal)),
        ],
        primary: 1,
        groups: vec!["Finance".into(), "Atlas core".into()],
    }
}

#[tokio::test]
async fn a_new_person_is_made_whole_from_the_form() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);

    let saved = contacts.save_form(None, &ada()).await.expect("saved");
    assert_eq!(saved.prior, None, "nobody was there before");

    let form = contacts.form_of(saved.person).await.unwrap().expect("read");
    assert_eq!(form.given, "Ada");
    assert_eq!(form.family, "Moreno");
    assert_eq!(form.show_as, ShowAs::LastFirst);
    assert_eq!(form.title, "Finance lead");
    assert_eq!(form.company, "Northwind Partners");
    assert_eq!(form.kind, ContactKind::Person);
    assert_eq!(form.groups, ["Atlas core", "Finance"]);
    let primary = &form.addresses[form.primary];
    assert_eq!(primary.address, "ada.moreno@example.com");
    let work = form
        .addresses
        .iter()
        .find(|a| a.address == "ada@example.org")
        .unwrap();
    assert_eq!(work.label, Some(AddressLabel::Work));

    // The inbox calls them what the style says.
    let person = contacts.get(saved.person).await.unwrap().unwrap();
    assert_eq!(person.display_name(), "Moreno, Ada");
    assert!(person.is_saved());
}

#[tokio::test]
async fn editing_changes_what_changed_and_hands_back_the_form_it_found() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let made = contacts.save_form(None, &ada()).await.unwrap().person;
    let before = contacts.form_of(made).await.unwrap().unwrap();

    let mut edited = before.clone();
    edited.given = "Adele".into();
    edited.kind = ContactKind::Company;
    edited.addresses.retain(|a| a.address != "ada@example.org");
    edited.addresses.push(row("a.moreno@example.net", None));
    edited.primary = 0;
    edited.groups = vec!["Finance".into()];
    let saved = contacts.save_form(Some(made), &edited).await.unwrap();
    assert_eq!(saved.prior.as_ref(), Some(&before));

    let after = contacts.form_of(made).await.unwrap().unwrap();
    assert_eq!(after.given, "Adele");
    assert_eq!(after.kind, ContactKind::Company);
    assert_eq!(after.groups, ["Finance"]);
    let addresses: Vec<&str> = after.addresses.iter().map(|a| a.address.as_str()).collect();
    assert!(addresses.contains(&"a.moreno@example.net"));
    assert!(!addresses.contains(&"ada@example.org"), "let go: {addresses:?}");
    // The released address belongs to nobody now, and is not deleted.
    let freed = contacts
        .by_address("ada@example.org")
        .await
        .unwrap()
        .map(|owner| owner.state);
    assert_ne!(freed, Some(ContactState::Live));

    // Saving what was found puts it back.
    contacts
        .save_form(Some(made), &saved.prior.unwrap())
        .await
        .unwrap();
    let restored = contacts.form_of(made).await.unwrap().unwrap();
    let mut expect = before.clone();
    let mut got = restored.clone();
    for form in [&mut expect, &mut got] {
        form.addresses.sort_by(|a, b| a.address.cmp(&b.address));
        for a in &mut form.addresses {
            a.id = None;
        }
    }
    got.primary = got
        .addresses
        .iter()
        .position(|a| a.address == restored.addresses[restored.primary].address)
        .unwrap();
    expect.primary = expect
        .addresses
        .iter()
        .position(|a| a.address == before.addresses[before.primary].address)
        .unwrap();
    assert_eq!(got, expect);
}

#[tokio::test]
async fn an_address_a_live_person_owns_refuses_the_whole_save() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    contacts
        .create(Some("Ben"), &[EmailAddress::new(None::<String>, "ben@example.net")])
        .await
        .unwrap();

    let mut form = ada();
    form.addresses.push(row("ben@example.net", None));
    let error = contacts.save_form(None, &form).await.unwrap_err();
    assert!(
        matches!(error, postio_storage::Error::AddressOwned { .. }),
        "{error:?}"
    );
    assert!(
        contacts.by_address("ada@example.org").await.unwrap().is_none(),
        "nothing was half-saved"
    );
}

#[tokio::test]
async fn a_form_that_cannot_be_saved_says_why() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let error = contacts
        .save_form(None, &ContactForm::blank())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Add at least one address"), "{error}");
}
