//! Capture, and `postio://` links, from the controller's side
//! (specs/009-focus-macos T108; spec 007 US15, screen 25).
//!
//! What GTK's `capture.rs` and the window's `open_capture` and `open_link`
//! did, asserted here so the Mac's capture window behaves the same: `t` and
//! `n` open it only when a vault is configured, and say how to name one
//! otherwise (C9); the text is the marker's sentence or the subject, and
//! `⌥S` swaps the subject in; the preview is the exact line written, with
//! the message's `postio://` link before the date (C21).

use chrono::{NaiveDate, TimeZone, Utc};
use postio_client::protocol::VaultPicture;
use postio_config::paths::Platform;
use postio_config::{Config, FocusConfig};
use postio_core::CommandId;
use postio_focus::{
    CaptureMode, CaptureView, Effect, FocusController, Host, Input, Intent, Policy, Reply, Request,
    RowFacts, Rows, SurfaceKind, Ticket,
};
use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen, MessageSummary, ThreadSummary};
use postio_model::{EmailAddress, MessageId, ThreadId};
use postio_ui::focus_list::FocusRow;
use postio_ui::keymap::KeyContext;

/// One conversation from Ada, asking for the slides by Friday.
struct List;

const ASK: &str = "Could you send the slides by Friday?";

fn friday() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 2).expect("a day")
}

impl Rows for List {
    fn len(&self) -> u32 {
        1
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.row(position).map(|row| RowFacts::of(&row))
    }
    fn position_of(&self, id: MessageId) -> Option<u32> {
        (id == MessageId::new(42)).then_some(0)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        (position == 0).then(|| {
            let at = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();
            let due = chrono::Local
                .from_local_datetime(&friday().and_hms_opt(12, 0, 0).unwrap())
                .unwrap()
                .to_utc();
            FocusRow::conversation(ThreadSummary {
                id: Some(ThreadId::new(7)),
                representative: MessageSummary {
                    id: MessageId::new(42),
                    thread: Some(ThreadId::new(7)),
                    from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
                    subject: Some("Quarterly review".to_owned()),
                    preview: None,
                    received_at: at,
                    seen: false,
                    flagged: false,
                    answered: false,
                    send_state: None,
                    send_at: None,
                    has_attachments: false,
                    thread_count: 1,
                    to: Vec::new(),
                },
                subject: Some("Quarterly review".to_owned()),
                participants: Vec::new(),
                message_count: 1,
                unread_count: 1,
                flagged: false,
                has_attachments: false,
                last_at: at,
                marker: Some(MarkerSummary {
                    kind: MarkerKind::Todo,
                    excerpt: Some(ASK.to_owned()),
                    when: Some(MarkerWhen::Due(due)),
                    answer: None,
                    cancelled: false,
                }),
                copies: Vec::new(),
            })
        })
    }
}

fn mac() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    let today = chrono::Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 9, 28)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        )
        .unwrap();
    let _ = focus.handle(Input::Clock(Some(today)));
    focus
}

/// `[focus]` with a vault when `vault`.
fn config(vault: bool) -> FocusConfig {
    let text = if vault {
        "[focus.vault]\npath = \"/vault.example\"\n"
    } else {
        ""
    };
    Config::from_toml_str(text).expect("a config").focus
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

fn drawn(effects: &[Effect]) -> Option<CaptureView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::OpenCapture(view) | Intent::Capture(view) => Some(*view),
            _ => None,
        })
}

/// The cursor on the row, `[focus]` as `config`.
fn ready(config: FocusConfig) -> FocusController {
    let mut focus = mac();
    let _ = focus.handle(Input::Config(config));
    let _ = focus.landed(&List, true);
    focus
}

#[test]
fn without_a_vault_t_and_n_say_how_to_name_one() {
    for command in [CommandId::CaptureTask, CommandId::CaptureNote] {
        let mut focus = ready(config(false));
        let effects = run(&mut focus, command);
        assert!(
            shown(&effects).iter().any(|intent| matches!(
                intent,
                Intent::Toast { text, .. } if text == postio_ui::capture::NO_VAULT
            )),
            "{command:?}: {effects:?}"
        );
        assert!(drawn(&effects).is_none(), "{command:?} opened nothing");
        assert!(asks(&effects).is_empty());
        assert_eq!(focus.key_context(), KeyContext::List);
    }
}

#[test]
fn with_a_vault_t_opens_capture_on_the_markers_sentence_and_its_day() {
    let mut focus = ready(config(true));
    let effects = run(&mut focus, CommandId::CaptureTask);
    let view = drawn(&effects).expect("capture opens");
    assert_eq!(view.mode, CaptureMode::Task);
    assert_eq!(view.text, ASK);
    assert_eq!(view.due, Some(friday()));
    assert!(
        view.picks.iter().any(|pick| pick.day == Some(friday())),
        "the mail's day among the quick picks"
    );
    assert_eq!(focus.key_context(), KeyContext::Capture);
    let link = postio_ui::links::message_uri(MessageId::new(42));
    let at_link = view
        .preview
        .find(&link)
        .expect("the line links the message");
    let at_due = view
        .preview
        .find("2026-10-02")
        .expect("the line names the day");
    assert!(
        at_link < at_due,
        "C21: the link before the date: {}",
        view.preview
    );
    assert!(asks(&effects).iter().any(|(_, request)| matches!(
        request,
        Request::Vault { subject, .. } if subject == "Quarterly review"
    )));
    // `n` from the list opens it as a note.
    let mut focus = ready(config(true));
    let view = drawn(&run(&mut focus, CommandId::CaptureNote)).expect("capture opens");
    assert_eq!(view.mode, CaptureMode::Note);
}

#[test]
fn alt_s_swaps_in_the_subject_and_the_preview_follows() {
    let mut focus = ready(config(true));
    let _ = run(&mut focus, CommandId::CaptureTask);
    let view = drawn(&run(&mut focus, CommandId::CaptureUseSubject)).expect("redrawn");
    assert_eq!(view.text, "Quarterly review");
    assert!(
        view.preview.contains("Quarterly review"),
        "{}",
        view.preview
    );
    let view = drawn(&focus.handle_on(
        Input::CaptureTyped {
            text: "Send the slides".to_owned(),
        },
        &List,
    ))
    .expect("redrawn");
    assert!(
        view.preview.starts_with("- [ ] Send the slides"),
        "{}",
        view.preview
    );
}

#[test]
fn the_vault_suggests_a_project_and_mod_return_writes_the_line() {
    let mut focus = ready(config(true));
    let effects = run(&mut focus, CommandId::CaptureTask);
    let [(ticket, Request::Vault { stamp, .. })] = asks(&effects)[..] else {
        panic!("the vault is read");
    };
    let project = postio_vault::Project {
        name: "Review".to_owned(),
        note: "Projects/Review.md".into(),
    };
    let picture = VaultPicture {
        projects: vec![project.clone()],
        suggestion: Some(postio_vault::Suggestion {
            project: project.clone(),
            reason: postio_vault::Reason::NamedInSubject("review".to_owned()),
        }),
        tasks_note: "Tasks.md".into(),
        tasks: Vec::new(),
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::Vault {
                stamp,
                answer: Ok(picture),
            },
        ),
        &List,
    );
    let view = drawn(&effects).expect("redrawn with the vault");
    assert_eq!(view.project, "Review");
    assert_eq!(
        view.project_title,
        postio_ui::capture::project_title(Some("Review"))
    );
    let effects = run(&mut focus, CommandId::CaptureWrite);
    let [
        (
            ticket,
            Request::CaptureTask {
                project: written,
                task,
                stamp,
            },
        ),
    ] = &asks(&effects)[..]
    else {
        panic!("mod+Return writes: {effects:?}");
    };
    assert_eq!(written.as_ref(), Some(&project));
    assert_eq!(task.text, ASK);
    assert_eq!(task.message, MessageId::new(42));
    assert_eq!(task.due, Some(friday()));
    let effects = focus.handle_on(
        Input::Reply(
            *ticket,
            Reply::Captured {
                stamp: *stamp,
                answer: Ok(()),
            },
        ),
        &List,
    );
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Capture)));
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. }
            if *text == postio_ui::capture::added(CaptureMode::Task, "Review", Some(friday()))
    )));
}

#[test]
fn esc_closes_capture_writing_nothing() {
    let mut focus = ready(config(true));
    let _ = run(&mut focus, CommandId::CaptureTask);
    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Capture)));
    assert!(asks(&effects).is_empty());
}

#[test]
fn a_postio_link_opens_its_message_and_a_stranger_says_why_not() {
    let mut focus = mac();
    let effects = focus.handle_on(Input::OpenLink("https://example.com/x".to_owned()), &List);
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if text == postio_ui::links::UNKNOWN
    )));
    let effects = focus.handle_on(Input::OpenLink("postio://message/42/".to_owned()), &List);
    let [(ticket, Request::FindMessage(found))] = asks(&effects)[..] else {
        panic!("the link is looked up: {effects:?}");
    };
    assert_eq!(found, MessageId::new(42));
    let row = List.row(0).and_then(|row| {
        row.as_conversation()
            .map(|row| row.summary.representative.clone())
    });
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::FoundMessage {
                message: found,
                row,
            },
        ),
        &List,
    );
    assert!(shown(&effects).contains(&Intent::OpenMessage {
        message: MessageId::new(42),
        index: 0,
        total: 1,
        host: Host::Own,
    }));
    let effects = focus.handle_on(Input::OpenLink("postio://message/43".to_owned()), &List);
    let [(ticket, Request::FindMessage(found))] = asks(&effects)[..] else {
        panic!("looked up");
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::FoundMessage {
                message: found,
                row: None,
            },
        ),
        &List,
    );
    assert!(shown(&effects).iter().any(|intent| matches!(
        intent,
        Intent::Toast { text, .. } if text == postio_ui::links::GONE
    )));
}
