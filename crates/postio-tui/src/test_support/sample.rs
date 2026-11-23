//! The states of the terminal that `examples/shot.rs` draws and the surface tests drive: the
//! inbox with one surface opened over it, reached as a person reaches it.

use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyModifiers};
use postio_model::mailbox::MailboxRole;
use postio_model::{AccountId, EmailAddress, MailboxId, MessageId};

use crate::app::{App, Input, update};
use crate::caps::Colour;
use crate::test_support::press_keys;

pub const STATES: &[&str] = &[
    "list",
    "reminder",
    "has-action",
    "selected",
    "toast",
    "error",
    "offline",
    "first-sync",
    "sign-in",
    "empty",
    "open",
    "pane",
    "bar",
    "bar-commands",
    "folders",
    "snooze",
    "remind",
    "label",
    "move",
    "keys",
    "compose",
    "filtered",
    "sweep",
    "digest",
    "digest-list",
    "digest-email",
    "rules",
    "rule",
    "capture",
];

pub fn state(state: &str, width: u16, height: u16) -> (App, Colour) {
    let state = match state {
        "reading" => "open",
        "undo" => "toast",
        "bulk" => "selected",
        other => other,
    };
    let nocolor = state.ends_with("-nocolor");
    let state = state.trim_end_matches("-nocolor");
    let state = if state == "nocolor" { "list" } else { state };
    let mut allowed = postio_ui::allowlist::RemoteImageAllowList::default();
    allowed.allow("newsletter@example.org");
    let mut app = crate::test_support::app((width, height)).with_allowlist(allowed);
    app.clock_for_tests(crate::test_support::now());

    crate::test_support::seed_places(
        &mut app,
        crate::places::Places {
            accounts: vec![crate::test_support::account()],
            folders: vec![
                {
                    let mut inbox = crate::test_support::folder(1, "INBOX", MailboxRole::Inbox, 3);
                    inbox.last_synced_at =
                        (state != "first-sync").then(|| crate::test_support::local(23, 11, 9));
                    inbox
                },
                crate::test_support::folder(2, "Archive", MailboxRole::Archive, 0),
                crate::test_support::folder(3, "Sent", MailboxRole::Sent, 0),
                crate::test_support::folder(4, "Drafts", MailboxRole::Drafts, 0),
                crate::test_support::folder(5, "Trash", MailboxRole::Trash, 0),
                crate::test_support::folder(6, "Projects", MailboxRole::Regular, 2),
                crate::test_support::folder(7, "Reading group", MailboxRole::Regular, 0),
                {
                    let mut year =
                        crate::test_support::folder(8, "Projects/2026", MailboxRole::Regular, 1);
                    year.name = "2026".into();
                    year.parent_id = Some(MailboxId::new(6));
                    year
                },
            ],
            counts: Vec::new(),
            saved: vec![
                crate::test_support::saved_search("waiting", "Waiting on reply", "is:unread"),
                crate::test_support::saved_search("atlas", "Atlas", "atlas"),
                crate::test_support::saved_search("receipts", "Receipts this month", "receipts"),
            ],
            features: crate::places::Features {
                filtering: true,
                digest_rules: 4,
                digests: crate::places::Rules(
                    postio_config::Config::from_toml_str(
                        "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@localfirst.example\", \"from:editor@ledger.example\"]\ncadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n\n\
                         [[focus.digests]]\nname = \"Receipts\"\nmatch = [\"from:billing@example.com\"]\ncadence = \"daily\"\nat = \"08:30\"\n\n\
                         [[focus.digests]]\nname = \"Harbor list\"\nmatch = [\"list:harbor.lists.example.org\"]\ncadence = \"monthly\"\nday = 1\nat = \"07:00\"\n",
                    )
                    .expect("rules")
                    .focus
                    .digests,
                ),
                capture: state == "capture",
                section: crate::places::Section::new(crate::test_support::with_model()),
                reading: if state == "pane" {
                    postio_config::Reading::Pane
                } else {
                    postio_config::Reading::Dialog
                },
                ..Default::default()
            },
        },
    );

    let rows = {
        use postio_model::listing::{Cadence, MarkerKind, MarkerSummary, MarkerWhen};
        use postio_ui::focus_list::{Conversation, Digest, FocusRow};
        let pills = |summary, names: &[&str]| {
            FocusRow::Conversation(Conversation {
                summary,
                labels: names
                    .iter()
                    .enumerate()
                    .map(|(at, name)| crate::test_support::label(at as i64 + 1, name))
                    .collect(),
            })
        };
        let invite = MarkerSummary {
            kind: MarkerKind::Invite,
            when: Some(MarkerWhen::Event {
                starts_at: crate::test_support::local(29, 10, 0),
                ends_at: crate::test_support::local(29, 10, 45),
            }),
            excerpt: None,
            answer: None,
            cancelled: false,
            notice: false,
        };
        let question = MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some(
                "Can you approve these by Friday so finance can close the quarter?".into(),
            ),
            answer: None,
            cancelled: false,
            notice: false,
        };
        let mut atlas = crate::test_support::unread(crate::test_support::marked(
            crate::test_support::conversation(
                3,
                "Ada Moreno",
                "Re: Atlas Q3 budget, final numbers",
                "Hi, the final Q3 numbers are in and the totals match what we discussed",
                crate::test_support::local(23, 11, 51),
            ),
            question,
        ));
        atlas.message_count = 3;
        atlas.has_attachments = true;
        vec![
            pills(
                crate::test_support::unread(crate::test_support::marked(
                    crate::test_support::conversation(
                        1,
                        "Grace Oyelaran",
                        "Invitation: Harbor design review",
                        "Tue 29 Sep 10:00-10:45, Room 3B, bring the harbor survey",
                        crate::test_support::local(23, 11, 2),
                    ),
                    invite,
                )),
                &["Harbor"],
            ),
            FocusRow::Digest(Digest {
                delivery: postio_model::ids::DeliveryId::new(1),
                rule: "Newsletters".into(),
                cadence: Some(Cadence::Weekly),
                count: 14,
                senders: Vec::new(),
                summary_line: Some("Summary of 14 messages from 6 senders: rail".into()),
                at: crate::test_support::local(23, 11, 0),
            }),
            pills(atlas, &["Atlas"]),
            FocusRow::conversation(crate::test_support::conversation(
                4,
                "Tomás Reyes",
                "Atlas staffing plan for Q4",
                "Sharing the draft before Monday's sync",
                crate::test_support::local(23, 10, 40),
            )),
            FocusRow::conversation(crate::test_support::conversation(
                5,
                "Marco Ruiz",
                "Cabinet order: please sign",
                "Attached the final order",
                crate::test_support::local(22, 14, 31),
            )),
        ]
    };
    let rows = if state == "reminder" {
        let fired =
            postio_ui::focus_list::FocusRow::surfaced(&postio_model::listing::Surfaced::Reminder {
                reminder: postio_model::ids::ReminderId::new(1),
                thread: postio_model::ids::ThreadId::new(9),
                since: crate::test_support::local(20, 9, 0),
                representative: crate::test_support::conversation(
                    9,
                    "Ines Varga",
                    "Proposal for the harbor survey",
                    "",
                    crate::test_support::local(20, 9, 0),
                )
                .representative,
                at: crate::test_support::local(23, 8, 0),
                position: 0,
            })
            .expect("a reminder is a row");
        std::iter::once(fired).chain(rows).collect()
    } else {
        rows
    };
    crate::test_support::show_focus(&mut app, rows);
    app.counts_for_tests(crate::test_support::drawing_counts());

    if matches!(state, "open" | "open-narrow" | "find" | "pane") {
        crate::test_support::open_message(
        &mut app,
        MessageId::new(1),
        vec![
            EmailAddress::new(Some("Tove Arnlund"), "tove@example.com"),
            EmailAddress::new(None::<String>, "gate-team@example.org"),
        ],
        postio_model::MessageBody {
            text: None,
            html: Some(
                "<p>Hi Tove,</p>\
                 <p>The overnight run held the gate at <strong>0.4 mm</strong>, well inside \
                 tolerance. Three things worth a look before Thursday:</p>\
                 <ul><li>the interlock fired twice at 03:10, both times on the east sensor;</li>\
                 <li>the second pump took <em>eleven seconds</em> longer to settle;</li>\
                 <li>the logs are attached, and the raw traces are on the <a href=\"https://example.com/traces\">shared drive</a>.</li></ul>\
                 <p>Can we go through them on Thursday?</p><p>Mira</p>\
                 <blockquote>On Tuesday, Tove wrote:<br>Could you run it overnight with the new \
                 sensor firmware?</blockquote>"
                    .into(),
            ),
        },
    );
    }

    let key = |app: &mut App, code: KeyCode, modifiers: KeyModifiers| {
        update(app, crate::test_support::key(code, modifiers));
    };
    let mut colour = Colour::TrueColor;
    match state {
        "bar" => {
            let opened = crate::test_support::type_text(&mut app, "/");
            let folders = app.folders_for_tests();
            crate::test_support::places_arrive(&mut app, &opened, &folders, &[], &[]);
            crate::test_support::type_text(&mut app, "arch");
        }
        "bar-commands" => {
            let opened = update(
                &mut app,
                crate::test_support::key(KeyCode::Char('k'), KeyModifiers::CONTROL),
            );
            let folders = app.folders_for_tests();
            crate::test_support::places_arrive(&mut app, &opened, &folders, &[], &[]);
            crate::test_support::type_text(&mut app, "ar");
        }
        "folders" => {
            let opened = crate::test_support::type_text(&mut app, "gl");
            let mut atlas = crate::test_support::label(7, "Atlas");
            atlas.color = Some("#3584e4".into());
            let folders = app.folders_for_tests();
            crate::test_support::places_arrive(
                &mut app,
                &opened,
                &folders,
                &[
                    (atlas, 12),
                    (crate::test_support::label(8, "Harbor"), 5),
                    (crate::test_support::label(9, "Kitchen reno"), 0),
                ],
                &[(AccountId::new(1), 2)],
            );
        }
        "snooze" => {
            crate::test_support::type_text(&mut app, "b");
        }
        "remind" => {
            crate::test_support::type_text(&mut app, "h");
        }
        "label" => {
            let opened = crate::test_support::type_text(&mut app, "l");
            let mut atlas = crate::test_support::label(7, "Atlas");
            atlas.color = Some("#3584e4".into());
            crate::test_support::labels_arrive(
                &mut app,
                &opened,
                vec![
                    atlas,
                    crate::test_support::label(8, "Harbor"),
                    crate::test_support::label(9, "Kitchen reno"),
                ],
                vec![
                    (postio_model::LabelId::new(7), 12),
                    (postio_model::LabelId::new(8), 5),
                ],
                &[postio_model::LabelId::new(8)],
            );
        }
        "move" => {
            let opened = crate::test_support::type_text(&mut app, "v");
            crate::test_support::folders_arrive(
                &mut app,
                &opened,
                crate::test_support::places().folders,
                vec![MailboxId::new(2), MailboxId::new(1)],
            );
        }
        "find" => {
            press_keys(&mut app, "<C-f>gate");
        }
        "keys" => {
            crate::test_support::type_text(&mut app, "?");
        }
        "compose" => {
            crate::test_support::type_text(&mut app, "c");
        }
        "privacy" => {
            key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
            while app
                .settings()
                .is_some_and(|settings| settings.current() != postio_ui::settings::Section::Privacy)
            {
                key(&mut app, KeyCode::Down, KeyModifiers::NONE);
            }
            let at = |hour, minute| Utc.with_ymd_and_hms(2026, 9, 24, hour, minute, 0).unwrap();
            let connection = |hour, minute, subsystem, host: &str, port, failed| {
                postio_model::egress::EgressEvent {
                    at: at(hour, minute),
                    subsystem,
                    account: None,
                    host: host.into(),
                    port,
                    outcome: if failed {
                        postio_model::egress::EgressOutcome::Failed
                    } else {
                        postio_model::egress::EgressOutcome::Connected
                    },
                }
            };
            use postio_model::egress::EgressSubsystem::{Imap, Smtp};
            update(
                &mut app,
                Input::Privacy {
                    log: postio_client::protocol::PrivacyLog {
                        activations: vec![postio_model::UnsubscribeActivation {
                            id: postio_model::ids::UnsubscribeActivationId::new(1),
                            account_id: AccountId::new(1),
                            list_identifier: "weekly.example.org".into(),
                            activated_at: at(9, 12),
                        }],
                        read_receipts: 3,
                    },
                    connections: vec![
                        connection(14, 58, Imap, "imap.example.com", 993, false),
                        connection(14, 41, Smtp, "smtp.example.com", 465, false),
                        connection(14, 40, Smtp, "smtp.example.com", 465, true),
                        connection(14, 12, Imap, "imap.example.com", 993, false),
                    ],
                },
            );
        }
        "undo" | "toast" => {
            update(
                &mut app,
                Input::Host(postio_core::Event::ActionCompleted {
                    description: "Archived 1 message".into(),
                    undoable: true,
                }),
            );
        }
        "error" => {
            update(
                &mut app,
                Input::Host(postio_core::Event::Error {
                    message: "The server refused the move: mailbox is read-only".into(),
                    account: None,
                }),
            );
        }
        "selected" => {
            for step in ['j', 'x', 'j', 'x', 'j', 'x', 'k'] {
                key(&mut app, KeyCode::Char(step), KeyModifiers::NONE);
            }
        }
        "has-action" => {
            key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE);
            let marked: Vec<_> = (0..2)
                .map(|_| ())
                .enumerate()
                .map(|(at, _)| {
                    postio_ui::focus_list::FocusRow::conversation(crate::test_support::marked(
                        crate::test_support::conversation(
                            at as i64 + 1,
                            ["Grace Oyelaran", "Ada Moreno"][at],
                            ["Invitation: Harbor design review", "Re: Atlas Q3 budget"][at],
                            "",
                            crate::test_support::local(23, 11, 2),
                        ),
                        postio_model::listing::MarkerSummary {
                            kind: postio_model::listing::MarkerKind::Question,
                            when: None,
                            excerpt: Some("Can you approve these by Friday?".into()),
                            answer: None,
                            cancelled: false,
                            notice: false,
                        },
                    ))
                })
                .collect();
            crate::test_support::show_scope(&mut app, postio_model::FocusScope::HasAction, marked);
        }
        "offline" | "first-sync" | "sign-in" | "empty" => {
            use postio_core::{ConnectionState, Event, FailureReason};
            let account = AccountId::new(1);
            let mut connected = |state| {
                update(
                    &mut app,
                    Input::Host(Event::ConnectionChanged { account, state }),
                );
            };
            match state {
                "offline" => connected(ConnectionState::Offline),
                "sign-in" => connected(ConnectionState::Failing {
                    reason: FailureReason::Auth,
                }),
                _ => connected(ConnectionState::Online),
            }
            let progress = |app: &mut App, done, total| {
                update(
                    app,
                    Input::Host(Event::SyncProgress {
                        account,
                        done,
                        total,
                    }),
                );
            };
            match state {
                "first-sync" => progress(&mut app, 3_200, 8_400),
                "empty" => {
                    progress(&mut app, 8_400, 8_400);
                    crate::test_support::show_focus(&mut app, Vec::new());
                }
                _ => {}
            }
        }
        "digest" | "digest-list" | "digest-email" => {
            use crate::test_support::{held, summary_of};
            key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
            let opening = update(
                &mut app,
                crate::test_support::key(KeyCode::Enter, KeyModifiers::NONE),
            );
            let messages = vec![
                held(
                    21,
                    "Harbor Weekly",
                    "Tide tables for October",
                    "The October tide tables are out",
                ),
                held(
                    22,
                    "Rail Notes",
                    "Timetable change",
                    "From the 5th the 8:10 runs at 8:15",
                ),
                held(
                    23,
                    "Town Hall",
                    "Bin collection",
                    "Collections move to Thursday",
                ),
            ];
            let mut summary = summary_of(&[
                (
                    "Your harbor",
                    "October's tide tables are published, and the east quay is closed for dredging on the 12th.",
                    21,
                ),
                (
                    "Getting around",
                    "The 8:10 train moves to 8:15 from the 5th.",
                    22,
                ),
                ("Your town", "Bins are collected on Thursdays now.", 23),
            ]);
            summary.statements[1].reference.excerpt = "the 8:10 runs at 8:15.".into();
            crate::test_support::serve_digest(&mut app, opening, messages, Some(summary));
            match state {
                "digest-list" => key(&mut app, KeyCode::Tab, KeyModifiers::NONE),
                "digest-email" => {
                    key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
                    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
                    update(
                        &mut app,
                        Input::Body {
                            message: MessageId::new(22),
                            answer: Ok(postio_client::protocol::Body::Ready {
                                body: postio_model::MessageBody {
                                    text: Some("Hello all,\n\nFrom the 5th the 8:10 runs at 8:15. Please plan around it.\n\nRail Notes".into()),
                                    html: None,
                                },
                                encoding_problems: false,
                            }),
                        },
                    );
                }
                _ => {}
            }
        }
        "capture" => {
            use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen};
            let todo = MarkerSummary {
                kind: MarkerKind::Todo,
                when: Some(MarkerWhen::Due(crate::test_support::local(25, 17, 0))),
                excerpt: Some("Please send the Atlas figures by Friday".into()),
                answer: None,
                cancelled: false,
                notice: false,
            };
            crate::test_support::show_focus(
                &mut app,
                vec![postio_ui::focus_list::FocusRow::conversation(
                    crate::test_support::marked(
                        crate::test_support::conversation(
                            1,
                            "Ada Moreno",
                            "Atlas figures for the board",
                            "",
                            crate::test_support::local(23, 9, 0),
                        ),
                        todo,
                    ),
                )],
            );
            let opening = update(
                &mut app,
                crate::test_support::key(KeyCode::Char('T'), KeyModifiers::NONE),
            );
            let project = |name: &str| postio_vault::Project {
                name: name.into(),
                note: std::path::PathBuf::from(format!("Projects/{name}.md")),
            };
            crate::test_support::serve_vault(
                &mut app,
                opening,
                postio_client::protocol::VaultPicture {
                    projects: vec![project("Atlas"), project("Garden")],
                    suggestion: Some(postio_vault::Suggestion {
                        project: project("Atlas"),
                        reason: postio_vault::Reason::NamedInSubject("atlas".into()),
                    }),
                    tasks_note: std::path::PathBuf::from("Tasks.md"),
                    tasks: Vec::new(),
                },
            );
        }
        "rules" | "rule" => {
            key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
            update(
                &mut app,
                crate::test_support::key(KeyCode::Char('r'), KeyModifiers::NONE),
            );
            update(
                &mut app,
                Input::Answer(crate::ask::Answer::Waiting {
                    names: vec![
                        "Newsletters".into(),
                        "Receipts".into(),
                        "Harbor list".into(),
                    ],
                    holds: Ok(vec![14, 3, 0]),
                }),
            );
            if state == "rule" {
                let effects = update(
                    &mut app,
                    crate::test_support::key(KeyCode::Enter, KeyModifiers::NONE),
                );
                crate::test_support::serve_focus(&mut app, effects, |request| match request {
                    postio_focus::Request::DigestPreview { stamp, .. } => {
                        Some(postio_focus::Reply::DigestPreview {
                            stamp: *stamp,
                            answer: Ok(postio_client::protocol::DigestPreview {
                                count: 9,
                                first: vec![
                                    crate::test_support::held(
                                        31,
                                        "Local First",
                                        "Local-first weekly 41",
                                        "",
                                    ),
                                    crate::test_support::held(
                                        32,
                                        "Local First",
                                        "Local-first weekly 40",
                                        "",
                                    ),
                                    crate::test_support::held(
                                        33,
                                        "Ledger",
                                        "The Ledger, September",
                                        "",
                                    ),
                                    crate::test_support::held(
                                        34,
                                        "Local First",
                                        "Local-first weekly 39",
                                        "",
                                    ),
                                ],
                            }),
                        })
                    }
                    _ => None,
                });
            }
        }
        "filtered" | "sweep" => {
            use crate::test_support::filtered_row;
            let filed = vec![
                filtered_row(11, "Forge", "notification", 23, 11, 2),
                filtered_row(12, "Ledger", "notification", 23, 10, 40),
                filtered_row(13, "Promo Weekly", "promotion", 23, 9, 5),
                filtered_row(14, "Forge", "notification", 22, 16, 0),
                filtered_row(15, "Rates Desk", "spam", 22, 8, 15),
                filtered_row(16, "Parcel Post", "shipping", 21, 12, 30),
            ];
            let tabs = [
                ("notification", 88),
                ("spam", 12),
                ("promotion", 41),
                ("receipt", 19),
                ("shipping", 14),
                ("social", 12),
            ];
            key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
            let effects = update(
                &mut app,
                crate::test_support::key(KeyCode::Char('f'), KeyModifiers::NONE),
            );
            crate::test_support::serve_filtered(&mut app, effects, &tabs, &filed);
            key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
            if state == "sweep" {
                let effects = update(
                    &mut app,
                    crate::test_support::key(KeyCode::Char('F'), KeyModifiers::NONE),
                );
                crate::test_support::serve_focus(&mut app, effects, |request| {
                    matches!(request, postio_focus::Request::SweepPreview)
                        .then_some(postio_focus::Reply::SweepPreview(Ok(7)))
                });
            }
        }
        _ => {}
    }

    if nocolor {
        colour = Colour::None;
    }
    (app, colour)
}
