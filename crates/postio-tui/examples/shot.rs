//! Render the terminal frontend with sample mail, as an SVG picture.
//!
//! For looking at the design, not for testing it: the screen is drawn by the
//! real `update` and `draw` into a test backend, then each cell is written out
//! with its colours, so a change to the look can be judged as a picture.
//!
//! ```bash
//! cargo run -p postio-tui --example shot -- /tmp/tui.svg [width] [height] [state]
//! magick /tmp/tui.svg /tmp/tui.png
//! ```
//!
//! `state` is what is open over the mail: `reading`, `open`, `pane` (from 128 columns) or `open-narrow` (the first message, in its frame; give `open-narrow` 76 columns), `bar`,
//! `bar-commands`, `bar-folder`, `folders`, `snooze`, `remind`, `label`, `move`, `keys` (the key map), `compose`, `undo` or `toast` (an undo offer on the bottom line),
//! `error`, `offline`, `first-sync`, `sign-in`, `empty`, `selected` or `bulk` (rows 2-4 marked, the cursor on row 3), or `nocolor`
//! `capture` (the capture sheet over a to-do), `rules` and `rule` (the digest rules, and the rule dialog over them), `digest`, `digest-list` and `digest-email` (a digest's window on its summary, its list and an email from a reference), `filtered`, `filtered-nocolor` and `sweep` (Filtered, and the sweep's question over it),
//! and `selected-nocolor`, `has-action` and `has-action-nocolor` (the same screens under `NO_COLOR`). Without one, the mail as it opens.
//!
//! Every name and address is fictional and on a reserved domain.

use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyModifiers};
use postio_model::mailbox::MailboxRole;
use postio_model::{AccountId, EmailAddress, MailboxId, MessageId};
use postio_tui::app::{App, Effect, Input, update};
use postio_tui::caps::{Background, Colour};
use postio_tui::test_support;
use postio_tui::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .unwrap_or_else(|| "/tmp/postio-tui.svg".to_owned());
    let width: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(160);
    let height: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(42);
    let state = args.next().unwrap_or_default();

    let mut allowed = postio_ui::allowlist::RemoteImageAllowList::default();
    allowed.allow("newsletter@example.org");
    let mut app = test_support::app((width, height)).with_allowlist(allowed);

    test_support::seed_places(
        &mut app,
        postio_tui::places::Places {
            accounts: vec![test_support::account()],
            folders: vec![
                {
                    let mut inbox = test_support::folder(1, "INBOX", MailboxRole::Inbox, 3);
                    inbox.last_synced_at =
                        (state != "first-sync").then(|| test_support::local(23, 11, 9));
                    inbox
                },
                test_support::folder(2, "Archive", MailboxRole::Archive, 0),
                test_support::folder(3, "Sent", MailboxRole::Sent, 0),
                test_support::folder(4, "Drafts", MailboxRole::Drafts, 0),
                test_support::folder(5, "Trash", MailboxRole::Trash, 0),
                test_support::folder(6, "Projects", MailboxRole::Regular, 2),
                test_support::folder(7, "Reading group", MailboxRole::Regular, 0),
                {
                    // A folder nested under another, as a server reports it.
                    let mut year =
                        test_support::folder(8, "Projects/2026", MailboxRole::Regular, 1);
                    year.name = "2026".into();
                    year.parent_id = Some(MailboxId::new(6));
                    year
                },
            ],
            counts: Vec::new(),
            saved: vec![
                test_support::saved_search("waiting", "Waiting on reply", "is:unread"),
                test_support::saved_search("atlas", "Atlas", "atlas"),
                test_support::saved_search("receipts", "Receipts this month", "receipts"),
            ],
            features: postio_tui::places::Features {
                filtering: true,
                digest_rules: 4,
                digests: postio_tui::places::Rules(
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
                reading: if state == "pane" {
                    postio_config::Reading::Pane
                } else {
                    postio_config::Reading::Dialog
                },
                ..Default::default()
            },
        },
    );

    // The inbox of terminal.md's drawing: an unread invitation with a pill,
    // a digest, a busy conversation, a read one and one from yesterday.
    let rows = {
        use postio_model::listing::{Cadence, MarkerKind, MarkerSummary, MarkerWhen};
        use postio_ui::focus_list::{Conversation, Digest, FocusRow};
        let pills = |summary, names: &[&str]| {
            FocusRow::Conversation(Conversation {
                summary,
                labels: names
                    .iter()
                    .enumerate()
                    .map(|(at, name)| test_support::label(at as i64 + 1, name))
                    .collect(),
            })
        };
        let invite = MarkerSummary {
            kind: MarkerKind::Invite,
            when: Some(MarkerWhen::Event {
                starts_at: test_support::local(29, 10, 0),
                ends_at: test_support::local(29, 10, 45),
            }),
            excerpt: None,
            answer: None,
            cancelled: false,
        };
        let question = MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some(
                "Can you approve these by Friday so finance can close the quarter?".into(),
            ),
            answer: None,
            cancelled: false,
        };
        let mut atlas = test_support::unread(test_support::marked(
            test_support::conversation(
                3,
                "Ada Moreno",
                "Re: Atlas Q3 budget, final numbers",
                "Hi, the final Q3 numbers are in and the totals match what we discussed",
                test_support::local(23, 11, 51),
            ),
            question,
        ));
        atlas.message_count = 3;
        atlas.has_attachments = true;
        vec![
            pills(
                test_support::unread(test_support::marked(
                    test_support::conversation(
                        1,
                        "Grace Oyelaran",
                        "Invitation: Harbor design review",
                        "Tue 29 Sep 10:00-10:45, Room 3B, bring the harbor survey",
                        test_support::local(23, 11, 2),
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
                at: test_support::local(23, 11, 0),
            }),
            pills(atlas, &["Atlas"]),
            FocusRow::conversation(test_support::conversation(
                4,
                "Tomás Reyes",
                "Atlas staffing plan for Q4",
                "Sharing the draft before Monday's sync",
                test_support::local(23, 10, 40),
            )),
            FocusRow::conversation(test_support::conversation(
                5,
                "Marco Ruiz",
                "Cabinet order: please sign",
                "Attached the final order",
                test_support::local(22, 14, 31),
            )),
        ]
    };
    test_support::show_focus(&mut app, rows);
    update(&mut app, Input::FocusCounts(test_support::drawing_counts()));

    // The first message open, for `reading`; otherwise the list.
    if matches!(state.as_str(), "reading" | "open" | "open-narrow" | "pane") {
        test_support::open_message(
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
        update(
            &mut app,
            test_support::key(KeyCode::Enter, KeyModifiers::NONE),
        );
    }

    let key = |app: &mut App, code: KeyCode, modifiers: KeyModifiers| {
        update(app, test_support::key(code, modifiers));
    };
    // Type `text`, and answer which search it asked last, if any.
    let typed = |app: &mut App, text: &str| {
        test_support::type_text(app, text)
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::BarSearch(ask) => Some(ask.sequence),
                _ => None,
            })
            .next_back()
    };
    let mut colour = Colour::TrueColor;
    match state.as_str() {
        "bar" => {
            // Words that name an operator are a search, answered with its hits.
            let sequence = typed(&mut app, "/from:ada tide").unwrap_or_default();
            let hit = |message: i64, thread: i64, subject: &str, snippet: &str| {
                postio_search::SearchHit {
                    message_id: MessageId::new(message),
                    thread_id: Some(postio_model::ThreadId::new(thread)),
                    mailbox_id: MailboxId::new(2),
                    subject: Some(subject.to_owned()),
                    from: Some(EmailAddress::new(Some("Ada Moreno"), "ada@example.com")),
                    received_at: test_support::local(23, 11, 51).with_timezone(&Utc),
                    snippet: snippet.to_owned(),
                    score: 0.0,
                }
            };
            let hits = vec![
                hit(
                    3,
                    3,
                    "Re: Atlas Q3 budget, final numbers",
                    "the \u{1}tide\u{2} tables for Q3",
                ),
                hit(4, 4, "Atlas staffing plan for Q4", "sharing the draft"),
            ];
            update(
                &mut app,
                Input::BarFound {
                    sequence,
                    found: Ok(Some(postio_client::protocol::Hits(
                        postio_search::SearchResults {
                            total_hits: 2,
                            hits,
                            total_hits_capped: false,
                            elapsed: std::time::Duration::from_millis(4),
                            corpus_complete: true,
                            suggestion: None,
                            instead: None,
                        },
                    ))),
                    held: Vec::new(),
                },
            );
        }
        "bar-commands" => {
            key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
            typed(&mut app, "ar");
        }
        "folders" => {
            typed(&mut app, "go");
            let mut atlas = test_support::label(7, "Atlas");
            atlas.color = Some("#3584e4".into());
            update(
                &mut app,
                Input::PlaceDetails(postio_tui::places::PlaceDetails {
                    labels: vec![
                        atlas,
                        test_support::label(8, "Harbor"),
                        test_support::label(9, "Kitchen reno"),
                    ],
                    label_counts: vec![
                        (postio_model::LabelId::new(7), 12),
                        (postio_model::LabelId::new(8), 5),
                    ],
                    correspondents: Vec::new(),
                    outbox: vec![(AccountId::new(1), 2)],
                }),
            );
        }
        "snooze" => {
            typed(&mut app, "s");
        }
        "remind" => {
            typed(&mut app, "h");
        }
        "label" => {
            typed(&mut app, "l");
            let mut atlas = test_support::label(7, "Atlas");
            atlas.color = Some("#3584e4".into());
            update(
                &mut app,
                Input::LabelPicker {
                    account: AccountId::new(1),
                    labels: vec![
                        atlas,
                        test_support::label(8, "Harbor"),
                        test_support::label(9, "Kitchen reno"),
                    ],
                    counts: vec![
                        (postio_model::LabelId::new(7), 12),
                        (postio_model::LabelId::new(8), 5),
                    ],
                    applied: [postio_model::LabelId::new(8)].into(),
                },
            );
        }
        "move" => {
            typed(&mut app, "m");
            update(
                &mut app,
                Input::RecentMoves(vec![MailboxId::new(6), MailboxId::new(2)]),
            );
        }
        "bar-folder" => {
            test_support::type_text(&mut app, "/in:arch");
        }
        "keys" => {
            typed(&mut app, "?");
        }
        "compose" => {
            typed(&mut app, "c");
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
        "selected" | "bulk" | "selected-nocolor" => {
            // Rows 2-4 marked, and the cursor back on row 3, inside them.
            for step in ['j', 'x', 'j', 'x', 'j', 'x', 'k'] {
                key(&mut app, KeyCode::Char(step), KeyModifiers::NONE);
            }
            if state == "selected-nocolor" {
                colour = Colour::None;
            }
        }
        "has-action" | "has-action-nocolor" => {
            // The toggle on: the list narrowed to the rows with a marker.
            key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE);
            let marked: Vec<_> = (0..2)
                .map(|_| ())
                .enumerate()
                .map(|(at, _)| {
                    postio_ui::focus_list::FocusRow::conversation(test_support::marked(
                        test_support::conversation(
                            at as i64 + 1,
                            ["Grace Oyelaran", "Ada Moreno"][at],
                            ["Invitation: Harbor design review", "Re: Atlas Q3 budget"][at],
                            "",
                            test_support::local(23, 11, 2),
                        ),
                        postio_model::listing::MarkerSummary {
                            kind: postio_model::listing::MarkerKind::Question,
                            when: None,
                            excerpt: Some("Can you approve these by Friday?".into()),
                            answer: None,
                            cancelled: false,
                        },
                    ))
                })
                .collect();
            test_support::show_scope(&mut app, postio_model::FocusScope::HasAction, marked);
            if state == "has-action-nocolor" {
                colour = Colour::None;
            }
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
            match state.as_str() {
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
            match state.as_str() {
                "first-sync" => progress(&mut app, 3_200, 8_400),
                "empty" => {
                    progress(&mut app, 8_400, 8_400);
                    test_support::show_focus(&mut app, Vec::new());
                }
                _ => {}
            }
        }
        "nocolor" => colour = Colour::None,
        "digest" | "digest-list" | "digest-email" => {
            use test_support::{held, summary_of};
            key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
            key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
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
            update(
                &mut app,
                Input::Answer(postio_tui::ask::Answer::Digest {
                    delivery: postio_model::ids::DeliveryId::new(1),
                    messages: Ok(messages),
                    summary: Ok(Some(summary)),
                }),
            );
            match state.as_str() {
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
                when: Some(MarkerWhen::Due(test_support::local(25, 17, 0))),
                excerpt: Some("Please send the Atlas figures by Friday".into()),
                answer: None,
                cancelled: false,
            };
            test_support::show_focus(
                &mut app,
                vec![postio_ui::focus_list::FocusRow::conversation(
                    test_support::marked(
                        test_support::conversation(
                            1,
                            "Ada Moreno",
                            "Atlas figures for the board",
                            "",
                            test_support::local(23, 9, 0),
                        ),
                        todo,
                    ),
                )],
            );
            key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
            let project = |name: &str| postio_vault::Project {
                name: name.into(),
                note: std::path::PathBuf::from(format!("Projects/{name}.md")),
            };
            update(
                &mut app,
                Input::Answer(postio_tui::ask::Answer::Vault(Ok(
                    postio_client::protocol::VaultPicture {
                        projects: vec![project("Atlas"), project("Garden")],
                        suggestion: Some(postio_vault::Suggestion {
                            project: project("Atlas"),
                            reason: postio_vault::Reason::NamedInSubject("atlas".into()),
                        }),
                        tasks_note: std::path::PathBuf::from("Tasks.md"),
                        tasks: Vec::new(),
                    },
                ))),
            );
        }
        "rules" | "rule" => {
            key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
            update(
                &mut app,
                test_support::key(KeyCode::Char('d'), KeyModifiers::NONE),
            );
            update(
                &mut app,
                Input::Answer(postio_tui::ask::Answer::Waiting {
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
                    test_support::key(KeyCode::Enter, KeyModifiers::NONE),
                );
                let generation = effects.iter().find_map(|effect| match effect {
                    Effect::Ask(postio_tui::ask::Ask::Preview { generation, .. }) => {
                        Some(*generation)
                    }
                    _ => None,
                });
                update(
                    &mut app,
                    Input::Answer(postio_tui::ask::Answer::Preview {
                        generation: generation.unwrap_or(1),
                        preview: Ok(postio_client::protocol::DigestPreview {
                            count: 9,
                            first: vec![
                                test_support::held(31, "Local First", "Local-first weekly 41", ""),
                                test_support::held(32, "Local First", "Local-first weekly 40", ""),
                                test_support::held(33, "Ledger", "The Ledger, September", ""),
                                test_support::held(34, "Local First", "Local-first weekly 39", ""),
                            ],
                        }),
                    }),
                );
            }
        }
        "filtered" | "filtered-nocolor" | "sweep" => {
            use test_support::filtered_row;
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
                test_support::key(KeyCode::Char('f'), KeyModifiers::NONE),
            );
            test_support::serve_filtered(&mut app, effects, &tabs, &filed);
            key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
            if state == "sweep" {
                key(&mut app, KeyCode::Char('F'), KeyModifiers::NONE);
                update(
                    &mut app,
                    Input::Answer(postio_tui::ask::Answer::SweepPreview(Ok(7))),
                );
            }
            if state == "filtered-nocolor" {
                colour = Colour::None;
            }
        }
        _ => {}
    }

    let (theme, _) = Theme::new(colour, Background::Dark, &Default::default());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let local = test_support::now();
    terminal
        .draw(|frame| {
            postio_tui::view::draw(frame, &app, &theme, local);
        })
        .unwrap();
    std::fs::write(&out, svg(terminal.backend().buffer())).unwrap();
    eprintln!("wrote {out}");
}

// A dark terminal palette in the spirit of GNOME Console's default, so the
// picture looks like a real terminal rather than raw ANSI primaries.
const BG: &str = "#1e1e24";
const FG: &str = "#deddda";

fn rgb(color: Color, fallback: &str) -> String {
    match color {
        Color::Reset => fallback.to_owned(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#1e1e24".into(),
        Color::Red => "#e5484d".into(),
        Color::Green => "#46a758".into(),
        Color::Yellow => "#f5d90a".into(),
        Color::Blue => "#3e63dd".into(),
        Color::Magenta => "#ab4aba".into(),
        Color::Cyan => "#05a2c2".into(),
        Color::Gray => "#9a9996".into(),
        Color::DarkGray => "#5e5c64".into(),
        Color::LightRed => "#ff6369".into(),
        Color::LightGreen => "#63c174".into(),
        Color::LightYellow => "#ffe629".into(),
        Color::LightBlue => "#849dff".into(),
        Color::LightMagenta => "#d19dff".into(),
        Color::LightCyan => "#4ccce6".into(),
        Color::White => "#ffffff".into(),
        Color::Indexed(i) => {
            let named = [
                "#1e1e24", "#e5484d", "#46a758", "#f5d90a", "#3e63dd", "#ab4aba", "#05a2c2",
                "#deddda", "#5e5c64", "#ff6369", "#63c174", "#ffe629", "#849dff", "#d19dff",
                "#4ccce6", "#ffffff",
            ];
            if (i as usize) < named.len() {
                named[i as usize].into()
            } else if i >= 232 {
                let v = 8 + (i - 232) * 10;
                format!("#{v:02x}{v:02x}{v:02x}")
            } else {
                let i = i - 16;
                let level = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
                format!(
                    "#{:02x}{:02x}{:02x}",
                    level(i / 36),
                    level((i / 6) % 6),
                    level(i % 6)
                )
            }
        }
    }
}

fn svg(buffer: &ratatui::buffer::Buffer) -> String {
    const W: f32 = 9.0;
    const H: f32 = 19.0;
    let area = buffer.area;
    let mut out = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}' font-family='DejaVu Sans Mono, monospace' font-size='15'>\
         <rect width='100%' height='100%' fill='{BG}'/>",
        f32::from(area.width) * W,
        f32::from(area.height) * H
    );
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (mut fg, mut bg) = (rgb(cell.fg, FG), rgb(cell.bg, BG));
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            if bg != BG {
                out += &format!(
                    "<rect x='{}' y='{}' width='{W}' height='{H}' fill='{bg}'/>",
                    f32::from(x) * W,
                    f32::from(y) * H
                );
            }
            let symbol = cell.symbol();
            if symbol.trim().is_empty() {
                continue;
            }
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "bold"
            } else {
                "normal"
            };
            let style = if cell.modifier.contains(Modifier::ITALIC) {
                "italic"
            } else {
                "normal"
            };
            let fg = if cell.modifier.contains(Modifier::DIM) {
                format!("{fg}99")
            } else {
                fg
            };
            let under = if cell.modifier.contains(Modifier::UNDERLINED) {
                " text-decoration='underline'"
            } else {
                ""
            };
            let escaped = symbol
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            out += &format!(
                "<text x='{}' y='{}' fill='{fg}' font-weight='{weight}' font-style='{style}'{under}>{escaped}</text>",
                f32::from(x) * W,
                f32::from(y) * H + 14.0
            );
        }
    }
    out + "</svg>"
}
