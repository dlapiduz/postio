//! The sync plan (specs/011-focus-contacts S12), over fake snapshots in both
//! directions. Nothing here touches an address book: the plan is a pure
//! function of what Postio has, what the other side has, and what was synced
//! last, and it never says "delete" and never overwrites a field both sides
//! changed. Every name and address is invented on a reserved domain.

use postio_contact_sync::{
    Change, ExternalRecord, Fields, Link, LocalRecord, Mode, Plan, Scope, Side, plan,
};
use postio_model::ContactKind;

fn fields(given: &str, family: &str, emails: &[&str], phones: &[&str]) -> Fields {
    Fields {
        given: given.to_owned(),
        family: family.to_owned(),
        emails: emails.iter().map(|e| (*e).to_owned()).collect(),
        phones: phones.iter().map(|p| (*p).to_owned()).collect(),
        organization: String::new(),
        title: String::new(),
    }
}

fn local(id: i64, fields: Fields) -> LocalRecord {
    LocalRecord {
        id,
        label: format!("{} {}", fields.given, fields.family)
            .trim()
            .to_owned(),
        fields,
        saved: true,
        kind: ContactKind::Person,
    }
}

fn external(id: &str, fields: Fields) -> ExternalRecord {
    ExternalRecord {
        id: id.to_owned(),
        label: format!("{} {}", fields.given, fields.family)
            .trim()
            .to_owned(),
        fields,
    }
}

fn run(locals: &[LocalRecord], externals: &[ExternalRecord], links: &[Link], mode: Mode) -> Plan {
    plan(locals, externals, links, Scope::PeopleAndCompanies, mode)
}

#[test]
fn a_shared_address_matches_and_makes_a_link_never_a_contact() {
    let l = local(1, fields("Ada", "Moreno", &["ada@example.org"], &[]));
    let e = external(
        "x1",
        fields(
            "Ada",
            "Moreno",
            &["ada@example.org", "ada@work.example"],
            &[],
        ),
    );
    let plan = run(&[l], &[e], &[], Mode::TwoWay);
    assert_eq!(plan.matches.len(), 1);
    assert_eq!(
        (plan.matches[0].local, plan.matches[0].external.as_str()),
        (1, "x1")
    );
    assert!(plan.creates.is_empty(), "a match is not a create");
    // They differ only in an address the other lacks: a set merges, no conflict.
    assert!(plan.conflicts.is_empty(), "{plan:?}");
}

#[test]
fn a_name_and_phone_match_when_no_address_is_shared_on_the_trailing_nine_digits() {
    let l = local(
        1,
        fields(
            "Ada",
            "Moreno",
            &["ada@example.org"],
            &["+1 (555) 010-0199"],
        ),
    );
    let e = external(
        "x1",
        fields(
            "Ada",
            "Moreno",
            &["a.moreno@example.net"],
            &["555 010 0199"],
        ),
    );
    let p = run(&[l], &[e], &[], Mode::TwoWay);
    assert_eq!(p.matches.len(), 1, "{p:?}");
    // The same phone with another name is somebody else.
    let other = local(
        2,
        fields("Ben", "Adeyemi", &["ben@example.org"], &["+1 555 010 0199"]),
    );
    let e2 = external(
        "x2",
        fields("Ada", "Moreno", &["a@example.net"], &["555 010 0199"]),
    );
    let p = run(&[other], &[e2], &[], Mode::TwoWay);
    assert!(p.matches.is_empty(), "{p:?}");
}

#[test]
fn more_than_one_candidate_on_either_side_is_a_question_not_a_link() {
    let l = local(1, fields("Ada", "Moreno", &["ada@example.org"], &[]));
    let e1 = external("x1", fields("Ada", "M", &["ada@example.org"], &[]));
    let e2 = external("x2", fields("A", "Moreno", &["ada@example.org"], &[]));
    let p = run(&[l], &[e1, e2], &[], Mode::TwoWay);
    assert!(p.matches.is_empty());
    assert_eq!(p.ambiguous.len(), 1, "{p:?}");
    assert_eq!(p.ambiguous[0].candidates.len(), 2);
    assert!(
        p.creates.is_empty(),
        "an ambiguous contact is not created either"
    );
}

#[test]
fn a_saved_contact_nobody_matches_is_a_create_only_in_two_way_mode() {
    let l = local(1, fields("Grace", "Oyelaran", &["grace@example.org"], &[]));
    let two = run(std::slice::from_ref(&l), &[], &[], Mode::TwoWay);
    assert_eq!(two.creates.len(), 1);
    assert_eq!(two.creates[0].local, 1);
    let read_only = run(&[l], &[], &[], Mode::ReadOnly);
    assert!(
        read_only.creates.is_empty(),
        "read only never plans a write to the other side"
    );
}

#[test]
fn an_unsaved_contact_never_syncs_and_a_record_nobody_has_is_left_alone() {
    let mut unsaved = local(1, fields("Ada", "Moreno", &["ada@example.org"], &[]));
    unsaved.saved = false;
    let stranger = external("x9", fields("Zed", "Zimmer", &["zed@example.org"], &[]));
    let p = run(&[unsaved], &[stranger], &[], Mode::TwoWay);
    assert!(p.creates.is_empty() && p.matches.is_empty());
    assert_eq!(
        p.left_alone, 1,
        "the other side's stranger stays there: {p:?}"
    );
}

#[test]
fn scope_keeps_lists_newsletters_and_robots_in_postio() {
    let mut newsletter = local(1, fields("Field", "Notes", &["news@field.example"], &[]));
    newsletter.kind = ContactKind::Newsletter;
    let mut company = local(
        2,
        fields("", "Northwind", &["office@northwind.example"], &[]),
    );
    company.kind = ContactKind::Company;
    let person = local(3, fields("Ada", "Moreno", &["ada@example.org"], &[]));
    let all = [newsletter, company, person];
    let people = plan(&all, &[], &[], Scope::People, Mode::TwoWay);
    assert_eq!(
        people.creates.iter().map(|c| c.local).collect::<Vec<_>>(),
        [3]
    );
    let both = plan(&all, &[], &[], Scope::PeopleAndCompanies, Mode::TwoWay);
    assert_eq!(
        both.creates.iter().map(|c| c.local).collect::<Vec<_>>(),
        [2, 3]
    );
    let everything = plan(&all, &[], &[], Scope::Everything, Mode::TwoWay);
    assert_eq!(everything.creates.len(), 3);
}

#[test]
fn when_only_one_side_changed_a_field_since_the_last_sync_that_side_wins() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mut mine = base.clone();
    mine.title = "Finance lead".to_owned();
    let theirs = base.clone();
    let link = Link {
        local: 1,
        external: "x1".into(),
        base,
    };
    let p = run(
        &[local(1, mine)],
        &[external("x1", theirs)],
        &[link],
        Mode::TwoWay,
    );
    assert!(p.conflicts.is_empty());
    assert_eq!(
        p.updates,
        [Change {
            local: 1,
            external: "x1".into(),
            label: "Ada Moreno".into(),
            side: Side::External,
            field: "title".into(),
            to: "Finance lead".into(),
        }]
    );
}

#[test]
fn a_change_on_their_side_is_a_pull_into_postio_and_read_only_pulls_only() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mine = base.clone();
    let mut theirs = base.clone();
    theirs.organization = "Northwind Partners".to_owned();
    let link = Link {
        local: 1,
        external: "x1".into(),
        base,
    };
    let p = run(
        &[local(1, mine)],
        &[external("x1", theirs)],
        std::slice::from_ref(&link),
        Mode::ReadOnly,
    );
    assert_eq!(p.updates.len(), 1);
    assert_eq!(p.updates[0].side, Side::Local);
    assert_eq!(p.updates[0].field, "organization");

    // And the other way round, read only plans nothing for their side.
    let base = link.base.clone();
    let mut mine = base.clone();
    mine.title = "Lead".to_owned();
    let p = run(
        &[local(1, mine)],
        &[external("x1", base)],
        &[link],
        Mode::ReadOnly,
    );
    assert!(p.updates.is_empty(), "{p:?}");
}

#[test]
fn both_sides_changing_one_field_to_different_values_is_a_question_and_writes_nothing() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mut mine = base.clone();
    mine.title = "Finance lead".to_owned();
    let mut theirs = base.clone();
    theirs.title = "Head of finance".to_owned();
    let link = Link {
        local: 1,
        external: "x1".into(),
        base,
    };
    let p = run(
        &[local(1, mine)],
        &[external("x1", theirs)],
        &[link],
        Mode::TwoWay,
    );
    assert!(
        p.updates.is_empty(),
        "nothing is written for that field meanwhile"
    );
    assert_eq!(p.conflicts.len(), 1);
    let c = &p.conflicts[0];
    assert_eq!(
        (c.field.as_str(), c.mine.as_str(), c.theirs.as_str()),
        ("title", "Finance lead", "Head of finance")
    );
}

#[test]
fn both_sides_changing_to_the_same_value_is_no_conflict() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mut mine = base.clone();
    mine.title = "Lead".to_owned();
    let theirs = mine.clone();
    let link = Link {
        local: 1,
        external: "x1".into(),
        base,
    };
    let p = run(
        &[local(1, mine)],
        &[external("x1", theirs)],
        &[link],
        Mode::TwoWay,
    );
    assert!(p.conflicts.is_empty() && p.updates.is_empty());
}

#[test]
fn a_first_sync_with_no_base_fills_what_one_side_lacks_and_asks_where_they_differ() {
    let mut mine = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    mine.title = "Finance lead".to_owned();
    let mut theirs = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    theirs.organization = "Northwind".to_owned();
    theirs.title = "CFO".to_owned();
    let p = run(
        &[local(1, mine)],
        &[external("x1", theirs)],
        &[],
        Mode::TwoWay,
    );
    assert_eq!(p.matches.len(), 1);
    let fields_set: Vec<(&str, Side)> = p
        .updates
        .iter()
        .map(|u| (u.field.as_str(), u.side))
        .collect();
    assert_eq!(
        fields_set,
        [("organization", Side::Local)],
        "what only they have comes in"
    );
    assert_eq!(p.conflicts.len(), 1, "the title differs with no base: ask");
}

#[test]
fn a_record_gone_from_the_other_side_is_a_question_never_a_delete() {
    let l = local(1, fields("Ada", "Moreno", &["ada@example.org"], &[]));
    let link = Link {
        local: 1,
        external: "gone".into(),
        base: l.fields.clone(),
    };
    let p = run(&[l], &[], &[link], Mode::TwoWay);
    assert_eq!(p.gone.len(), 1, "{p:?}");
    assert!(p.deletes.is_empty(), "deletions are never automatic");
    assert!(
        p.creates.is_empty(),
        "and a gone contact is not quietly recreated"
    );
}

#[test]
fn the_summary_says_what_would_happen_in_the_words_the_review_reads() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mut mine = base.clone();
    mine.title = "Lead".to_owned();
    let mut theirs = base.clone();
    theirs.title = "CFO".to_owned();
    let linked = Link {
        local: 1,
        external: "x1".into(),
        base,
    };
    let locals = [
        local(1, mine),
        local(2, fields("Grace", "Oyelaran", &["grace@example.org"], &[])),
        local(3, fields("Ben", "Adeyemi", &["ben@example.org"], &[])),
    ];
    let p = run(&locals, &[external("x1", theirs)], &[linked], Mode::TwoWay);
    assert_eq!(p.summary(), "Would add 2, update 0, ask about 1, delete 0");
    assert_eq!(p.counts(), (2, 0, 1, 0));
}

#[test]
fn the_report_names_people_and_the_counts_never_do() {
    let l = local(2, fields("Grace", "Oyelaran", &["grace@example.org"], &[]));
    let p = run(&[l], &[], &[], Mode::TwoWay);
    let report = p.report();
    assert!(
        report.iter().any(|line| line.contains("Grace Oyelaran")),
        "{report:?}"
    );
    assert!(!p.summary().contains("Grace"), "the summary is counts only");
}

#[test]
fn an_update_line_names_the_person_never_an_id() {
    let base = fields("Ada", "Moreno", &["ada@example.org"], &[]);
    let mut theirs = base.clone();
    theirs.organization = "Northwind Partners".to_owned();
    let link = Link {
        local: 93,
        external: "x1".into(),
        base: base.clone(),
    };
    let p = run(
        &[local(93, base)],
        &[external("x1", theirs)],
        &[link],
        Mode::TwoWay,
    );
    let report = p.report();
    assert!(
        report
            .iter()
            .any(|l| l.starts_with("Update Ada Moreno in Postio")),
        "{report:?}"
    );
    assert!(!report.iter().any(|l| l.contains("93")), "{report:?}");
}
