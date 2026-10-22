//! The two keys and the one question (specs/011-focus-contacts S12). The
//! address book is a trait; every test here, and the demo, uses a fake that
//! counts what reaches it. Nothing is asked of a person's address book unless
//! they turned sync on, and nothing is written unless the build says so and
//! they confirmed this very plan.

use postio_contact_sync::{
    Authorization, Confirmation, ExternalAddressBook, ExternalRecord, FakeAddressBook, Fields,
    LocalRecord, Mode, Scope, SyncError, dry_run, enable, write,
};
use postio_model::ContactKind;

fn fields(given: &str, address: &str) -> Fields {
    Fields {
        given: given.to_owned(),
        family: String::new(),
        emails: [address.to_owned()].into_iter().collect(),
        phones: Vec::new(),
        organization: String::new(),
        title: String::new(),
    }
}

fn locals() -> Vec<LocalRecord> {
    vec![LocalRecord {
        id: 1,
        label: "Grace".into(),
        fields: fields("Grace", "grace@example.org"),
        saved: true,
        kind: ContactKind::Person,
    }]
}

fn book(state: Authorization) -> FakeAddressBook {
    FakeAddressBook::new(
        state,
        vec![ExternalRecord {
            id: "x1".into(),
            label: "Ada".into(),
            fields: fields("Ada", "ada@example.org"),
        }],
    )
}

fn plan_for(book: &mut FakeAddressBook) -> postio_contact_sync::Plan {
    dry_run(
        book,
        &locals(),
        &[],
        Scope::PeopleAndCompanies,
        Mode::TwoWay,
    )
    .expect("a plan")
}

#[test]
fn a_dry_run_reads_a_snapshot_and_nothing_else() {
    let mut book = book(Authorization::Authorized);
    let plan = plan_for(&mut book);
    assert_eq!(plan.counts().0, 1, "Grace would be added");
    assert_eq!(book.snapshots(), 1);
    assert_eq!(book.applied(), 0, "a dry run never writes");
    assert_eq!(book.prompts(), 0, "and never asks for access");
}

#[test]
fn without_permission_a_dry_run_says_so_and_never_prompts_or_reads() {
    for state in [Authorization::NotDetermined, Authorization::Denied] {
        let mut book = book(state);
        let error = dry_run(&mut book, &locals(), &[], Scope::People, Mode::TwoWay).unwrap_err();
        assert_eq!(error, SyncError::NotAuthorized(state));
        assert_eq!(
            (book.prompts(), book.snapshots(), book.applied()),
            (0, 0, 0),
            "{state:?}"
        );
    }
}

#[test]
fn access_is_asked_for_only_by_enable_which_is_the_users_turning_sync_on() {
    let mut book = book(Authorization::NotDetermined).granting(Authorization::Authorized);
    assert_eq!(enable(&mut book), Authorization::Authorized);
    assert_eq!(book.prompts(), 1);
    // Already decided: turning it on again does not prompt a second time.
    assert_eq!(enable(&mut book), Authorization::Authorized);
    assert_eq!(book.prompts(), 1);
    // Denied is a final state until the person enables it again.
    let mut denied = book_denied();
    assert_eq!(enable(&mut denied), Authorization::Denied);
    assert_eq!(
        denied.prompts(),
        0,
        "the system will not ask again, and neither do we"
    );
}

fn book_denied() -> FakeAddressBook {
    book(Authorization::Denied)
}

#[test]
fn writes_need_both_keys_the_build_flag_and_the_confirmation_of_this_plan() {
    let mut fake = book(Authorization::Authorized);
    let plan = plan_for(&mut fake);
    let confirmed = Some(Confirmation::of(&plan));

    // Flag off, confirmed: nothing.
    assert_eq!(
        write(&mut fake, &plan, confirmed, false),
        Err(SyncError::WritesOff)
    );
    // Flag on, not confirmed: nothing.
    assert_eq!(
        write(&mut fake, &plan, None, true),
        Err(SyncError::NotConfirmed)
    );
    // Both off: nothing.
    assert!(write(&mut fake, &plan, None, false).is_err());
    assert_eq!(
        fake.applied(),
        0,
        "with either key off nothing reached the store"
    );

    // A confirmation of another plan is not this plan's.
    let mut other = book(Authorization::Authorized);
    let mut other_locals = locals();
    other_locals[0].fields.title = "Different".into();
    let other_plan = dry_run(&mut other, &other_locals, &[], Scope::People, Mode::TwoWay).unwrap();
    assert_ne!(Confirmation::of(&plan), Confirmation::of(&other_plan));
    assert_eq!(
        write(&mut fake, &plan, Some(Confirmation::of(&other_plan)), true),
        Err(SyncError::NotConfirmed)
    );
    assert_eq!(fake.applied(), 0);

    // Only with both keys does anything reach the (fake) store.
    assert_eq!(write(&mut fake, &plan, confirmed, true), Ok(()));
    assert_eq!(fake.applied(), 1);
}

#[test]
fn the_build_flag_ships_off() {
    const {
        assert!(
            !postio_contact_sync::CONTACT_SYNC_WRITES,
            "flipping it is a reviewed commit, not this one"
        );
    }
}

#[test]
fn a_read_only_mode_plan_cannot_be_written_even_with_both_keys() {
    let mut fake = book(Authorization::Authorized);
    let plan = dry_run(&mut fake, &locals(), &[], Scope::People, Mode::ReadOnly).unwrap();
    assert_eq!(
        write(&mut fake, &plan, Some(Confirmation::of(&plan)), true),
        Err(SyncError::ReadOnly)
    );
    assert_eq!(fake.applied(), 0);
}

#[test]
fn the_trait_is_all_a_real_store_has_to_be() {
    // A compile-time check that the fake is a stand-in for the trait and the
    // trait has no other way in.
    fn takes(_: &mut dyn ExternalAddressBook) {}
    takes(&mut book(Authorization::Authorized));
}
