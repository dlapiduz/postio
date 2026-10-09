//! The digest window and the digest rule dialog, from the controller's side
//! (specs/009-focus-macos T108; spec 007 US10, US13, US14; screens 22-24).
//!
//! What GTK's `digest.rs`, `rule_dialog.rs` and the window's digest handlers
//! did, asserted here so the Mac's digest window behaves the same: a digest
//! opens on its summary when the person has brought a model and on its plain
//! list otherwise (C6); `]`/`[` walk the references; Return opens a
//! reference's email in the digest's own window and Esc comes back to the
//! same reference; `⇧A` archives the delivery; `D` stops digesting a sender
//! only once confirmed; `d` opens the rule dialog filled in. On the Mac a
//! message from the digest's list opens in the digest's window too (M4).

use chrono::Utc;
use postio_config::paths::Platform;
use postio_config::{Config, FocusConfig};
use postio_core::{Command, CommandId, MessageTarget};
use postio_focus::{
    DigestPage, DigestView, Effect, FocusController, Host, Input, Intent, Policy, Reply, Request,
    RowFacts, Rows, RuleView, SurfaceKind, Ticket,
};
use postio_model::listing::{Cadence, MessageSummary, ThreadSummary};
use postio_model::summary::{DigestSummary, SummaryReference, SummaryStatement};
use postio_model::{DeliveryId, EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::{Digest, FocusRow};
use postio_ui::keymap::KeyContext;

const DELIVERY: i64 = 40;

/// A message of the digest, or of the list: `id`, from `address`.
fn message(id: i64, name: &str, address: &str, subject: &str) -> MessageSummary {
    MessageSummary {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(id + 1000)),
        from: Some(EmailAddress::new(Some(name), address)),
        subject: Some(subject.to_owned()),
        preview: None,
        received_at: Utc::now(),
        seen: false,
        flagged: false,
        answered: false,
        send_state: None,
        send_at: None,
        has_attachments: false,
        thread_count: 1,
        to: Vec::new(),
    }
}

/// The list: a digest's row on top, then two conversations.
struct List;

impl List {
    fn digest() -> Digest {
        Digest {
            delivery: DeliveryId::new(DELIVERY),
            rule: "Newsletters".to_owned(),
            cadence: Some(Cadence::Weekly),
            count: 3,
            senders: vec![EmailAddress::new(Some("Ledger"), "news@ledger.example")],
            summary_line: None,
            at: Utc::now(),
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        3
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.row(position).map(|row| RowFacts::of(&row))
    }
    fn position_of(&self, id: MessageId) -> Option<u32> {
        (0..3).find(|at| self.facts(*at).is_some_and(|row| row.id == id))
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        match position {
            0 => Some(FocusRow::Digest(List::digest())),
            1 | 2 => {
                let id = 100 + i64::from(position);
                let summary = message(id, "Ada", "ada@example.com", "Plans");
                Some(FocusRow::conversation(ThreadSummary {
                    id: summary.thread,
                    subject: summary.subject.clone(),
                    representative: summary,
                    participants: Vec::new(),
                    message_count: 1,
                    unread_count: 1,
                    flagged: false,
                    has_attachments: false,
                    last_at: Utc::now(),
                    marker: None,
                    copies: Vec::new(),
                }))
            }
            _ => None,
        }
    }
}

fn mac() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn linux() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Freedesktop))
}

/// `[focus]` with the Newsletters rule, and a model when `model`.
fn config(model: bool) -> FocusConfig {
    let mut text =
        "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@ledger.example\"]\n\
                    cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n"
            .to_owned();
    if model {
        text.push_str(
            "[focus.model]\nendpoint = \"http://127.0.0.1:11434/v1\"\nmodel = \"a-small-model\"\n",
        );
    }
    Config::from_toml_str(&text).expect("a config").focus
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

fn run(focus: &mut FocusController, command: CommandId) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), &List)
}

fn drawn(effects: &[Effect]) -> Option<DigestView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Digest(view) => Some(*view),
            _ => None,
        })
}

/// What the delivery holds: three messages from Ledger and Forge.
fn held() -> Vec<MessageSummary> {
    vec![
        message(1, "Ledger", "news@ledger.example", "Rates"),
        message(2, "Forge", "builds@forge.example", "Build 12"),
        message(3, "Ledger", "news@ledger.example", "Trains"),
    ]
}

/// A summary citing the first and third messages.
fn summary() -> DigestSummary {
    let statement = |topic: &str, number: u32, message: i64, excerpt: &str| SummaryStatement {
        topic: topic.to_owned(),
        text: format!("Something about {topic}."),
        reference: SummaryReference {
            number,
            message: MessageId::new(message),
            excerpt: excerpt.to_owned(),
        },
    };
    DigestSummary {
        statements: vec![
            statement("Rates", 1, 1, "rates held"),
            statement("Trains", 2, 3, "the line opens"),
        ],
        messages: 3,
        senders: 2,
    }
}

/// `Return` on the digest's row with `[focus]` as `config`, and the reads
/// answered: what the window draws once they land.
fn open(focus: &mut FocusController, config: FocusConfig) -> (Vec<Effect>, Vec<Effect>) {
    let _ = focus.handle(Input::Config(config));
    let _ = focus.landed(&List, true);
    let opening = run(focus, CommandId::OpenMessage);
    let mut landed = Vec::new();
    for (ticket, request) in asks(&opening) {
        let Request::DigestRead { stamp, summary, .. } = request else {
            continue;
        };
        landed.extend(focus.handle_on(
            Input::Reply(
                ticket,
                Reply::DigestRead {
                    stamp,
                    rows: Ok(held()),
                    summary: summary.then(summary_of),
                },
            ),
            &List,
        ));
    }
    (opening, landed)
}

fn summary_of() -> DigestSummary {
    summary()
}

#[test]
fn a_digest_opens_on_its_summary_with_a_model_and_on_its_list_without() {
    let mut focus = mac();
    let (opening, landed) = open(&mut focus, config(true));
    assert!(shown(&opening).contains(&Intent::OpenDigest {
        row: MessageId::new(-DELIVERY)
    }));
    assert!(
        asks(&opening).iter().any(|(_, request)| matches!(
            request,
            Request::DigestRead { delivery, summary: true, .. } if delivery.get() == DELIVERY
        )),
        "with a model the summary is read: {opening:?}"
    );
    assert_eq!(focus.key_context(), KeyContext::Digest);
    let view = drawn(&landed).expect("drawn once read");
    assert_eq!(view.page, DigestPage::Summary);
    assert_eq!(view.focused_reference, Some(0));
    assert_eq!(view.title, "Weekly \u{b7} Newsletters");
    let numbers: Vec<u32> = view
        .topics
        .iter()
        .flat_map(|topic| topic.statements.iter().map(|statement| statement.number))
        .collect();
    assert_eq!(numbers, [1, 2], "numbered references");

    // C6: no model, no summary asked for, and the list is what opens.
    let mut focus = mac();
    let (opening, landed) = open(&mut focus, config(false));
    assert!(
        asks(&opening)
            .iter()
            .any(|(_, request)| matches!(request, Request::DigestRead { summary: false, .. }))
    );
    let view = drawn(&landed).expect("drawn once read");
    assert_eq!(view.page, DigestPage::List);
    assert_eq!(view.rows.len(), 3);
    assert_eq!(view.focused, Some(0));
}

#[test]
fn brackets_step_the_references_and_stop_at_the_ends() {
    let mut focus = mac();
    let _ = open(&mut focus, config(true));
    let view = drawn(&run(&mut focus, CommandId::NextReference)).expect("redrawn");
    assert_eq!(view.focused_reference, Some(1));
    // The list's focus follows the reference: `D` and `U` act on its mail.
    assert_eq!(view.focused, Some(2));
    let view = drawn(&run(&mut focus, CommandId::NextReference)).expect("redrawn");
    assert_eq!(view.focused_reference, Some(1), "clamped at the last");
    let view = drawn(&run(&mut focus, CommandId::PrevReference)).expect("redrawn");
    assert_eq!(view.focused_reference, Some(0));
}

#[test]
fn return_opens_the_references_email_in_the_digest_and_esc_returns_to_it() {
    for mut focus in [mac(), linux()] {
        let _ = open(&mut focus, config(true));
        let _ = run(&mut focus, CommandId::NextReference);
        let effects = run(&mut focus, CommandId::OpenMessage);
        assert!(
            shown(&effects).contains(&Intent::OpenMessage {
                message: MessageId::new(3),
                index: 2,
                total: 3,
                host: Host::Digest,
            }),
            "{effects:?}"
        );
        assert!(
            !shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Digest)),
            "the digest's own window"
        );
        let view = drawn(&effects).expect("the email page");
        assert_eq!(view.page, DigestPage::Email);
        let email = view.email.expect("the email");
        assert_eq!(email.message, MessageId::new(3));
        assert_eq!(email.excerpt.as_deref(), Some("the line opens"));
        assert_eq!(
            email.banner.as_deref(),
            Some(postio_ui::digest::cited_banner(2).as_str())
        );
        let back = drawn(&run(&mut focus, CommandId::Back)).expect("back to the summary");
        assert_eq!(back.page, DigestPage::Summary);
        assert_eq!(back.focused_reference, Some(1), "the same reference");
        assert_eq!(focus.key_context(), KeyContext::Digest);
    }
}

#[test]
fn shift_a_archives_the_whole_delivery() {
    let mut focus = mac();
    let _ = open(&mut focus, config(true));
    let effects = run(&mut focus, CommandId::ArchiveThread);
    assert!(
        asks(&effects).iter().any(|(_, request)| *request
            == Request::Post(Command::ArchiveDigest {
                delivery: DeliveryId::new(DELIVERY),
                archived: true,
            })),
        "{effects:?}"
    );
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Digest)));
}

#[test]
fn shift_d_stops_digesting_the_sender_only_once_confirmed() {
    let mut focus = mac();
    let _ = open(&mut focus, config(false));
    let _ = run(&mut focus, CommandId::NextMessage);
    let effects = run(&mut focus, CommandId::StopDigestingSender);
    assert!(asks(&effects).is_empty(), "nothing before the answer");
    let confirm = shown(&effects)
        .into_iter()
        .find_map(|intent| match intent {
            Intent::Confirm(confirm) => Some(confirm),
            _ => None,
        })
        .expect("D asks first");
    assert_eq!(
        confirm.heading,
        postio_ui::focus_target::stop_digesting_title("builds@forge.example")
    );
    assert_eq!(confirm.body, postio_ui::focus_target::STOP_DIGESTING_BODY);
    let effects = focus.handle_on(Input::Confirmed(confirm.token), &List);
    assert_eq!(
        asks(&effects)
            .into_iter()
            .map(|(_, asked)| asked)
            .collect::<Vec<_>>(),
        vec![Request::Post(Command::StopDigestingSender {
            target: MessageTarget::Messages(vec![MessageId::new(2)]),
            stopped: true,
            kept: None,
        })]
    );
    // An answer for a question since replaced does nothing.
    assert!(
        focus
            .handle_on(Input::Confirmed(confirm.token), &List)
            .is_empty()
    );
}

#[test]
fn u_unsubscribes_from_the_focused_messages_list() {
    let mut focus = mac();
    let _ = open(&mut focus, config(false));
    let effects = run(&mut focus, CommandId::Unsubscribe);
    let [(ticket, Request::Unsubscribe(message))] = asks(&effects)[..] else {
        panic!("U asks: {effects:?}");
    };
    assert_eq!(message, MessageId::new(1));
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::Unsubscribed(Ok("Ledger".to_owned()))),
        &List,
    );
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if *text == postio_ui::focus_target::unsubscribed("Ledger")
    )));
}

#[test]
fn on_the_mac_a_message_from_the_digests_list_opens_in_the_digests_window() {
    let mut focus = mac();
    let _ = open(&mut focus, config(false));
    let _ = run(&mut focus, CommandId::NextMessage);
    let effects = run(&mut focus, CommandId::OpenMessage);
    assert!(
        shown(&effects).contains(&Intent::OpenMessage {
            message: MessageId::new(2),
            index: 1,
            total: 3,
            host: Host::Digest,
        }),
        "{effects:?}"
    );
    assert!(!shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Digest)));
    assert_eq!(drawn(&effects).expect("in place").page, DigestPage::Email);
    assert_eq!(focus.key_context(), KeyContext::Digest);
    // `j` walks the digest's messages in place.
    let effects = run(&mut focus, CommandId::NextMessage);
    assert!(shown(&effects).contains(&Intent::OpenMessage {
        message: MessageId::new(3),
        index: 2,
        total: 3,
        host: Host::Digest,
    }));
    let back = drawn(&run(&mut focus, CommandId::Back)).expect("back to the list");
    assert_eq!(back.page, DigestPage::List);

    // Linux stacks: its reading dialog opens over the digest.
    let mut focus = linux();
    let _ = open(&mut focus, config(false));
    let effects = run(&mut focus, CommandId::OpenMessage);
    assert!(shown(&effects).contains(&Intent::OpenMessage {
        message: MessageId::new(1),
        index: 0,
        total: 3,
        host: Host::Own,
    }));
    assert_eq!(focus.key_context(), KeyContext::Reader);
    let _ = run(&mut focus, CommandId::Back);
    assert_eq!(focus.key_context(), KeyContext::Digest);
}

#[test]
fn a_digest_takes_no_verb_meant_for_the_list_behind_it() {
    let mut focus = mac();
    let _ = open(&mut focus, config(false));
    assert_eq!(focus.key_context(), KeyContext::Digest);
    // The list's cursor on a conversation, behind the digest.
    let _ = focus.handle_on(Input::Point(1), &List);
    for command in [CommandId::Archive, CommandId::Delete, CommandId::ToggleRead] {
        let effects = run(&mut focus, command);
        assert!(asks(&effects).is_empty(), "{command:?}: {effects:?}");
    }
}

fn rule(effects: &[Effect]) -> Option<RuleView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::OpenRule(view) | Intent::Rule(view) => Some(*view),
            _ => None,
        })
}

#[test]
fn d_opens_the_rule_dialog_filled_in_with_the_senders_and_a_preview() {
    let mut focus = mac();
    let _ = focus.landed(&List, true);
    let _ = run(&mut focus, CommandId::NextMessage);
    let effects = run(&mut focus, CommandId::DigestRule);
    let view = rule(&effects).expect("the dialog opens");
    assert_eq!(view.heading, postio_ui::digest::new_rule_heading(1));
    assert_eq!(view.from.as_deref(), Some("ada@example.com"));
    assert_eq!(view.schedule, postio_ui::digest::Schedule::new_rule());
    assert_eq!(view.create, "Create");
    let [(ticket, Request::DigestPreview { queries, stamp, .. })] = &asks(&effects)[..] else {
        panic!("the preview is read: {effects:?}");
    };
    assert_eq!(queries, &["from:ada@example.com".to_owned()]);
    assert_eq!(focus.key_context(), KeyContext::List, "a dialog's keys");
    let preview = postio_client::protocol::DigestPreview {
        count: 9,
        first: vec![message(5, "Ada", "ada@example.com", "Plans")],
    };
    let effects = focus.handle_on(
        Input::Reply(
            *ticket,
            Reply::DigestPreview {
                stamp: *stamp,
                answer: Ok(preview),
            },
        ),
        &List,
    );
    let view = rule(&effects).expect("redrawn with the preview");
    assert_eq!(
        view.preview_heading.as_deref(),
        Some(postio_ui::digest::preview_heading(9).as_str())
    );
    assert_eq!(view.more.as_deref(), Some("and 8 more"));

    // Create writes the rule, closes, and says so.
    let effects = focus.handle_on(Input::RuleCreate, &List);
    let [
        (
            ticket,
            Request::SaveDigestRule {
                replacing,
                draft,
                stamp,
            },
        ),
    ] = &asks(&effects)[..]
    else {
        panic!("Create writes: {effects:?}");
    };
    assert_eq!(*replacing, None);
    assert_eq!(draft.name, "Ada");
    let effects = focus.handle_on(
        Input::Reply(
            *ticket,
            Reply::RuleSaved {
                stamp: *stamp,
                answer: Ok(()),
            },
        ),
        &List,
    );
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Dialog)));
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if *text == postio_ui::focus_target::rule_saved("Ada")
    )));
}

#[test]
fn d_in_a_digest_edits_its_rule() {
    let mut focus = mac();
    let _ = open(&mut focus, config(false));
    let view = rule(&run(&mut focus, CommandId::DigestRule)).expect("the dialog opens");
    assert_eq!(
        view.heading,
        postio_ui::digest::edit_rule_heading("Newsletters")
    );
    assert_eq!(view.create, "Save");
    assert_eq!(view.from.as_deref(), Some("from:news@ledger.example"));
}
