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
//! `state` is what is open over the mail: `search`, `palette`, `keys` (the
//! cheat sheet), `compose`, `undo` (an undo offer on the status line),
//! `error`, `selected` (rows 2-4 marked, the cursor on row 3), or `nocolor`
//! and `selected-nocolor` (the same screens under `NO_COLOR`). Without one, the mail as it opens.
//!
//! Every name and address is fictional and on a reserved domain.

use chrono::{Duration, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{AccountId, EmailAddress, ListScope, MailboxId, MessageId};
use postio_tui::app::{App, Effect, Input, update};
use postio_tui::caps::{Background, Colour};
use postio_tui::input::Keys;
use postio_tui::row::Row;
use postio_tui::theme::Theme;
use postio_ui::paging::Fetch;
use postio_ui::terminal::SafeText;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

const MESSAGES: &[(&str, &str, &str, bool, bool, bool)] = &[
    (
        "Mira Castell",
        "Tide gate interlock report",
        "The overnight run held at 0.4 mm; I've attached the logs and",
        true,
        true,
        true,
    ),
    (
        "Tove Arnlund",
        "Re: The analytical engine's second table",
        "Agreed on the ordering. One thought on the carries before we",
        true,
        false,
        false,
    ),
    (
        "Ines Okonkwo-Hale",
        "Trajectory numbers for Thursday",
        "Rechecked by hand -- the margins are wider than the model says",
        false,
        false,
        true,
    ),
    (
        "Joss Remy",
        "Reading group: morphogenesis",
        "Next week's paper is short. Bring questions about the reaction",
        false,
        false,
        false,
    ),
    (
        "Petra Vancel",
        "Priority display, revised",
        "The alarm path now drops the lowest-priority jobs first; see",
        true,
        false,
        false,
    ),
    (
        "Oren Baptiste",
        "On the cruelty of teaching",
        "A draft, for your comments. I would rather hear the objections",
        false,
        true,
        false,
    ),
    (
        "Nell Ashgrove",
        "Spanning tree, the poem",
        "It rhymes, mostly. The algorithm is less forgiving than the",
        false,
        false,
        false,
    ),
    (
        "Wren Hallory",
        "Substitution, again",
        "Your counter-example is a good one, and I think it is about",
        false,
        false,
        false,
    ),
    (
        "Dora Quimby",
        "Compiler meeting moved to 3pm",
        "Same room. Coffee will be there early this time, I promise",
        false,
        false,
        false,
    ),
    (
        "Silas Fenwold",
        "Errata, volume 4B",
        "Two cheques in the post. The second one is for the index,",
        false,
        false,
        false,
    ),
    (
        "Lark Imrie",
        "Frequency hopping patent notes",
        "Scanned the originals; the piano-roll figures are legible",
        false,
        false,
        true,
    ),
    (
        "Teodor Pask",
        "Lambda notation question",
        "Is the quote form necessary here, or is it only a",
        false,
        false,
        false,
    ),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .unwrap_or_else(|| "/tmp/postio-tui.svg".to_owned());
    let width: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(160);
    let height: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(42);
    let state = args.next().unwrap_or_default();

    let keys = Keys::new(&postio_core::Keymap::resolve(&Default::default())).0;
    let mut allowed = postio_ui::allowlist::RemoteImageAllowList::default();
    allowed.allow("newsletter@example.org");
    let mut app = App::new((width, height), keys).with_allowlist(allowed);

    let mut account =
        postio_model::Account::new("Ada", EmailAddress::new(None::<String>, "ada@example.com"));
    account.id = AccountId::new(1);
    account.enabled = true;
    let folder = |id: i64, name: &str, role: MailboxRole, unread: u32| {
        let mut folder = Mailbox::new(account.id, name, None);
        folder.id = MailboxId::new(id);
        folder.role = role;
        folder.selectable = true;
        folder.counts.unread = unread;
        folder
    };
    update(
        &mut app,
        Input::Sidebar(postio_tui::sidebar::Contents {
            accounts: vec![account.clone()],
            folders: vec![
                folder(1, "INBOX", MailboxRole::Inbox, 3),
                folder(2, "Archive", MailboxRole::Archive, 0),
                folder(3, "Sent", MailboxRole::Sent, 0),
                folder(4, "Drafts", MailboxRole::Drafts, 0),
                folder(5, "Trash", MailboxRole::Trash, 0),
                folder(6, "Projects", MailboxRole::Regular, 2),
                folder(7, "Reading group", MailboxRole::Regular, 0),
                {
                    // A folder nested under another, as a server reports it.
                    let mut year = folder(8, "Projects/2026", MailboxRole::Regular, 1);
                    year.name = "2026".into();
                    year.parent_id = Some(MailboxId::new(6));
                    year
                },
            ],
            counts: Vec::new(),
            saved: Vec::new(),
        }),
    );

    let now = Utc.with_ymd_and_hms(2026, 9, 24, 15, 0, 0).unwrap();
    let rows: Vec<Row> = MESSAGES
        .iter()
        .enumerate()
        .map(
            |(at, (from, subject, preview, unread, flagged, attachment))| Row {
                id: MessageId::new(at as i64 + 1),
                thread: None,
                is_thread: false,
                from: SafeText::new(from),
                address: Some(format!(
                    "{}@example.com",
                    from.split(' ').next().unwrap().to_lowercase()
                )),
                subject: SafeText::new(subject),
                preview: SafeText::new(preview),
                when: now - Duration::hours(at as i64 * 7 + 1),
                unread: *unread,
                flagged: *flagged,
                attachment: *attachment,
                count: 1,
            },
        )
        .collect();
    let total = rows.len() as u32;
    let mut pending = update(
        &mut app,
        Input::Opened {
            scope: ListScope::Mailbox(MailboxId::new(1)),
            total,
        },
    );
    while let Some(effect) = pending.pop() {
        if let Effect::Fetch {
            generation,
            page,
            fetch: Fetch::Scope(request),
        } = effect
        {
            let page_rows = rows
                .iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .cloned()
                .collect();
            pending.extend(update(
                &mut app,
                Input::Page {
                    generation,
                    page,
                    rows: Ok(postio_ui::paging::Page {
                        total,
                        rows: page_rows,
                    }),
                },
            ));
        }
    }

    // The first message open in the reader.
    let first = MessageId::new(1);
    update(&mut app, Input::Rested(first));
    update(
        &mut app,
        Input::Addressed {
            message: first,
            to: vec![
                EmailAddress::new(Some("Tove Arnlund"), "tove@example.com"),
                EmailAddress::new(None::<String>, "gate-team@example.org"),
            ],
        },
    );
    update(
        &mut app,
        Input::Body {
            message: first,
            answer: Ok(postio_client::protocol::Body::Ready {
                body: postio_model::MessageBody {
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
                encoding_problems: false,
            }),
        },
    );

    let key = |app: &mut App, code: KeyCode, modifiers: KeyModifiers| {
        update(app, Input::Key(KeyEvent::new(code, modifiers)));
    };
    // Type `text`, and answer which search it asked last, if any.
    let typed = |app: &mut App, text: &str| {
        let mut asked = None;
        for c in text.chars() {
            for effect in update(app, Input::Key(KeyEvent::from(KeyCode::Char(c)))) {
                if let Effect::Search { sequence, .. } = effect {
                    asked = Some(sequence);
                }
            }
        }
        asked
    };
    let mut colour = Colour::TrueColor;
    match state.as_str() {
        "search" => {
            let sequence = typed(&mut app, "/tide").unwrap_or_default();
            update(
                &mut app,
                Input::Found {
                    sequence,
                    found: Ok(Some(postio_client::protocol::Found {
                        ids: vec![MessageId::new(1), MessageId::new(3), MessageId::new(5)],
                        hits: 3,
                        capped: false,
                        corpus_complete: true,
                        elapsed: std::time::Duration::from_millis(7),
                    })),
                },
            );
            use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};
            update(
                &mut app,
                Input::Facets {
                    sequence,
                    facets: Some(Facets {
                        scopes: vec![
                            ScopeCount {
                                scope: Scope::AllMail,
                                hits: 3,
                            },
                            ScopeCount {
                                scope: Scope::Inbox,
                                hits: 2,
                            },
                            ScopeCount {
                                scope: Scope::Lists,
                                hits: 0,
                            },
                        ],
                        refinements: vec![
                            Refinement {
                                token: "is:unread".into(),
                                hits: 2,
                            },
                            Refinement {
                                token: "from:mira".into(),
                                hits: 1,
                            },
                        ],
                    }),
                },
            );
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        }
        "palette" => {
            key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
            typed(&mut app, "ar");
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
        "undo" => {
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
                }),
            );
        }
        "selected" | "selected-nocolor" => {
            // Rows 2-4 marked, and the cursor back on row 3, inside them.
            for step in ['j', 'x', 'j', 'x', 'j', 'x', 'k'] {
                key(&mut app, KeyCode::Char(step), KeyModifiers::NONE);
            }
            if state == "selected-nocolor" {
                colour = Colour::None;
            }
        }
        "nocolor" => colour = Colour::None,
        _ => {}
    }

    let (theme, _) = Theme::new(colour, Background::Dark, &Default::default());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let local = chrono::Local.from_utc_datetime(&now.naive_utc());
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
