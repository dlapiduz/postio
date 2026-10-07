//! Every surface terminal.md draws, under `NO_COLOR` and under the mouse.
//!
//! Each case is one state of `test_support::sample`, drawn with no colour at
//! all. It must show the marks "Colour and marks" lists for what the state
//! holds, every control it draws must take a click that does what the control's
//! key does, the wheel must scroll what scrolls, and with `[tui] mouse = false`
//! no pointer input does anything while the keys still do.

use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::app::{App, Effect};
use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

use crate::test_support::{
    buffer, click_at, hits_of, mouse_on, press_keys, sample, screen, wheel_at,
};
use crate::view::hit::Target;

/// What a state's screen must show without colour.
enum Mark {
    /// This text is on the screen.
    Has(&'static str),
    /// This text is on the screen, and every cell of its first occurrence
    /// carries `Modifier`.
    Styled(&'static str, Modifier),
    /// As `Styled`, for the `nth` occurrence of the text.
    StyledAt(&'static str, usize, Modifier),
    /// This text is on the screen and no cell of its first occurrence carries
    /// `Modifier`.
    Plain(&'static str, Modifier),
}

/// A control the screen draws: where it is, and the keys it stands for.
#[derive(Clone)]
struct Control {
    /// Text on the screen, whose first cell is clicked.
    label: &'static str,
    /// Which occurrence of the label, from the top left.
    nth: usize,
    /// Keys that bring the app where the key means what the click means.
    prep: &'static str,
    /// The control's key.
    keys: &'static str,
}

const fn ctl(label: &'static str, keys: &'static str) -> Control {
    Control {
        label,
        nth: 0,
        prep: "",
        keys,
    }
}

impl Control {
    const fn nth(mut self, nth: usize) -> Control {
        self.nth = nth;
        self
    }

    const fn after(mut self, prep: &'static str) -> Control {
        self.prep = prep;
        self
    }
}

/// A state of the sample and what is true of it.
struct Case {
    surface: &'static str,
    state: &'static str,
    size: (u16, u16),
    /// Keys pressed after the state is reached.
    then: &'static str,
    marks: Vec<Mark>,
    controls: Vec<Control>,
    /// Text over which the wheel scrolls something at this size.
    wheel: Option<&'static str>,
    /// Whether the surface has the whole screen's keyboard and pointer, the
    /// top bar and the bottom line too, rather than all but those.
    holds: bool,
    /// Whether what is under the surface takes no click, as under a frame.
    modal: bool,
}

const fn case(surface: &'static str, state: &'static str, size: (u16, u16)) -> Case {
    Case {
        surface,
        state,
        size,
        then: "",
        marks: Vec::new(),
        controls: Vec::new(),
        wheel: None,
        holds: true,
        modal: true,
    }
}

const BOLD: Modifier = Modifier::BOLD;
const REVERSED: Modifier = Modifier::REVERSED;

/// The top bar's and the strip's controls.
fn chrome() -> Vec<Control> {
    vec![
        ctl("Compose c", "c"),
        ctl("⌕ Search", "/"),
        ctl("ctrl+k", "<C-k>"),
        ctl("? keys", "?"),
        ctl("Inbox ▾", "go"),
        ctl("⚑ Has action", "!"),
        ctl("186 filtered today", "gf"),
        ctl("4 digest rules", "gd"),
    ]
}

fn cases() -> Vec<Case> {
    use Mark::{Has, Plain, Styled, StyledAt};
    const ITALIC: Modifier = Modifier::ITALIC;
    vec![
        Case {
            marks: vec![
                Has("▌"),
                Has("●"),
                Has("≡"),
                Has("⎘"),
                Has("⚑"),
                Styled("Grace Oyelaran", BOLD),
                Plain("Tomás Reyes", BOLD),
                Styled("Invite", BOLD),
                Styled("Can you approve", ITALIC),
            ],
            controls: [
                chrome(),
                vec![
                    ctl("Accept y", "y"),
                    ctl("Decline Y", "Y"),
                    ctl("Reply e", "e").after("jj"),
                    ctl("Tomás Reyes", "jjj"),
                ],
            ]
            .concat(),
            holds: false,
            ..case("inbox", "list", (120, 36))
        },
        Case {
            marks: vec![Has("▌↺"), Has("No reply"), Styled("No reply", BOLD)],
            holds: false,
            ..case("fired reminder", "reminder", (120, 36))
        },
        Case {
            // Ten lines of mail in a nine-line list, so there is a line to
            // scroll.
            wheel: Some("Grace Oyelaran"),
            holds: false,
            ..case("inbox, scrolling", "reminder", (120, 12))
        },
        Case {
            marks: vec![
                Has("⚑"),
                Styled("⚑ Has action", REVERSED),
                Has("Showing 7 of 312"),
            ],
            controls: vec![ctl("⚑ Has action", "!"), ctl("! again to show all", "!")],
            holds: false,
            ..case("has-action toggle", "has-action", (120, 36))
        },
        Case {
            then: "jj",
            marks: vec![
                Has("✓"),
                Has("▌"),
                Styled("Tomás Reyes", REVERSED),
                Plain("Marco Ruiz", REVERSED),
                Has("2 selected"),
            ],
            controls: vec![
                ctl("Archive a", "a"),
                ctl("Snooze s", "s"),
                ctl("Mark read r", "r"),
                ctl("Digest these… d", "d"),
                ctl("Label l", "l"),
                ctl("Move m", "m"),
                ctl("Delete Del", "<Del>"),
                ctl("x toggle", "x"),
                ctl("J K extend", "J"),
                ctl("K extend", "K"),
                ctl("Esc clear", "<Esc>"),
            ],
            holds: false,
            ..case("bulk bar", "selected", (160, 36))
        },
        Case {
            marks: vec![Has("✓ Archived 1 message"), Has("Undo ctrl+z")],
            controls: vec![ctl("Undo ctrl+z", "<C-z>")],
            holds: false,
            ..case("toast", "toast", (120, 36))
        },
        Case {
            marks: vec![Has("✕ The server refused")],
            holds: false,
            ..case("error toast", "error", (120, 36))
        },
        Case {
            marks: vec![Has("○ Offline"), Styled("You're offline", BOLD)],
            controls: vec![ctl("Retry now F5", "<F5>")],
            holds: false,
            ..case("offline banner", "offline", (120, 36))
        },
        Case {
            marks: vec![
                Has("Syncing"),
                Has("━"),
                Has("─"),
                Styled("First sync", BOLD),
            ],
            holds: false,
            ..case("first sync banner", "first-sync", (120, 36))
        },
        Case {
            marks: vec![Has("✕ Can't sign in"), Styled("Can't sign in", BOLD)],
            controls: vec![ctl("Update password…", "")],
            holds: false,
            ..case("sign-in banner", "sign-in", (120, 36))
        },
        Case {
            marks: vec![Styled("Inbox is empty", BOLD)],
            controls: vec![
                ctl("Filtered g f", "gf"),
                ctl("Archive g r", "gr"),
                ctl("Compose c", "c").nth(1),
            ],
            holds: false,
            ..case("empty inbox", "empty", (120, 36))
        },
        Case {
            marks: vec![
                Has("╭"),
                Styled("Invitation: Harbor design review", BOLD),
                Has("●Harbor"),
                Has("▸ 2 quoted lines"),
                Styled("Invite", BOLD),
                Styled("Dismiss -", REVERSED),
                Styled("Accept y", REVERSED),
            ],
            controls: vec![
                ctl("↑ k", "k").after("j"),
                ctl("↓ j", "j"),
                ctl("Esc ✕", "<Esc>"),
                ctl("Reply e", "e"),
                ctl("Reply all E", "E"),
                ctl("Forward f", "f"),
                ctl("Archive a", "a"),
                ctl("Snooze s", "s"),
                ctl("Remind h", "h"),
                ctl("More .", "."),
                ctl("+ Label l", "l"),
                ctl("Accept y", "y"),
                ctl("Decline Y", "Y").nth(1),
                ctl("Dismiss -", "-"),
            ],
            holds: false,
            ..case("open message frame", "open", (120, 36))
        },
        Case {
            marks: vec![
                Has("⌕ gate"),
                Has("1 of 2"),
                StyledAt("gate", 0, REVERSED),
                StyledAt("gate", 0, Modifier::UNDERLINED),
                StyledAt("gate", 1, REVERSED),
            ],
            controls: vec![
                ctl("ctrl+g next", "<C-g>"),
                ctl("shift+F3 previous", "<S-F3>"),
                ctl("Esc close", "<Esc>"),
            ],
            holds: false,
            ..case("find in the open message", "find", (120, 36))
        },
        Case {
            then: ".",
            marks: vec![Has("▌Label"), Has("╭ More")],
            controls: vec![ctl("Move", "m"), ctl("Delete", "<Del>")],
            holds: false,
            ..case("the More menu", "open", (120, 36))
        },
        Case {
            then: ".",
            marks: vec![Has("▌Label"), Has("╭ More")],
            controls: vec![ctl("Move", "m")],
            holds: false,
            modal: false,
            ..case("the More menu beside the list", "pane", (140, 36))
        },
        Case {
            wheel: Some("Hi Tove"),
            holds: false,
            ..case("open message frame, scrolling", "open", (120, 22))
        },
        Case {
            marks: vec![
                Has("▌"),
                Has("│"),
                Styled("Invite", BOLD),
                Styled("Dismiss -", REVERSED),
            ],
            controls: vec![
                ctl("↑ k", "k").after("j"),
                ctl("↓ j", "j"),
                ctl("Esc ✕", "<Esc>"),
                ctl("Reply e", "e"),
                ctl("Reply all E", "E"),
                ctl("Forward f", "f"),
                ctl("Archive a", "a"),
                ctl("Snooze s", "s"),
                ctl("Remind h", "h"),
                ctl("More .", "."),
                ctl("+ Label l", "l"),
                ctl("Accept y", "y").nth(1),
                ctl("Decline Y", "Y").nth(1),
                ctl("Dismiss -", "-"),
            ],
            holds: false,
            modal: false,
            ..case("reading pane", "pane", (140, 36))
        },
        Case {
            then: "<Esc>",
            marks: vec![
                Has("To-do"),
                Styled("To-do", BOLD),
                Styled("Please send", ITALIC),
            ],
            controls: vec![ctl("Task t", "t"), ctl("Snooze s", "s")],
            holds: false,
            ..case("a to-do's row", "capture", (120, 36))
        },
        Case {
            then: "<Tab><Tab>line 1<Enter>line 2<Enter>line 3<Enter>line 4<Enter>line 5<Enter>line 6<Enter>line 7<Enter>line 8<Enter>line 9<Enter>line 10<Enter>line 11<Enter>line 12<Enter>line 13<Enter>line 14<Enter>line 15<Enter>line 16<Enter>line 17<Enter>line 18<Enter>line 19<Enter>line 20<Enter>line 21<Enter>line 22<Enter>line 23<Enter>line 24<Enter>line 25<Enter>line 26<Enter>line 27<Enter>line 28<Enter>line 29<Enter>line 30<Enter>line 31<Enter>line 32<Enter>line 33<Enter>line 34<Enter>line 35<Enter>line 36<Enter>line 37<Enter>line 38<Enter>line 39<Enter>line 40<Enter>",
            wheel: Some("line 2"),
            holds: false,
            ..case("composer frame, scrolling", "compose", (120, 24))
        },
        Case {
            marks: vec![Has("╭")],
            controls: vec![
                ctl("Detach alt+o", "<A-o>"),
                ctl("Esc ✕", "<Esc>"),
                ctl("Send alt+s", "<A-s>"),
                ctl("Send later alt+S", "<A-S>"),
                ctl("Attach alt+a", "<A-a>"),
                ctl("Remind ctrl+h", "<C-h>"),
            ],
            holds: false,
            ..case("composer frame", "compose", (120, 36))
        },
        Case {
            marks: vec![Has("╭"), Has("▌")],
            controls: vec![
                ctl("Waiting on reply alt+1", "<A-1>"),
                ctl("Atlas alt+2", "<A-2>"),
                ctl("Receipts this month alt+3", "<A-3>"),
                ctl("ctrl+s saves", "<C-s>"),
                ctl("Esc", "<Esc>"),
                ctl("↵ run", "<Enter>"),
                ctl("> commands only", "<C-k>"),
                ctl("from:ada", "<Tab>").nth(2),
                ctl("tide", "<Tab><Tab>").nth(2),
            ],
            ..case("command bar", "bar", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("g r")],
            controls: vec![ctl("Waiting on reply alt+1", "<A-1>")],
            wheel: Some("Archive thread"),
            ..case("command bar commands", "bar-commands", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("●"), Has("g i")],
            wheel: Some("Drafts"),
            ..case("folders box", "folders", (120, 36))
        },
        Case {
            wheel: Some("Tomorrow morning"),
            ..case("snooze picker, scrolling", "snooze", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("╭")],
            controls: vec![ctl("Tomorrow morning", "2"), ctl("tab", "<Tab>")],
            ..case("snooze picker", "snooze", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("╭")],
            controls: vec![ctl("In 2 working days", "2"), ctl("tab", "<Tab>")],
            ..case("remind picker", "remind", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("✓ applied"), Has("●")],
            ..case("label picker", "label", (120, 36))
        },
        Case {
            marks: vec![Has("▌"), Has("╭")],
            ..case("move picker", "move", (120, 36))
        },
        Case {
            marks: vec![Has("╭"), Has("✕")],
            controls: vec![ctl("✕", "?")],
            wheel: Some("Next message"),
            holds: false,
            ..case("key map", "keys", (120, 36))
        },
        Case {
            marks: vec![
                Has("▌"),
                Has("1 All"),
                Styled("1 All", Modifier::UNDERLINED),
            ],
            controls: vec![
                ctl("‹ Inbox g i", "gi"),
                ctl("Sweep the inbox… F", "F"),
                ctl("R restore + never filter sender", "R"),
                ctl("g i inbox", "gi"),
                ctl("2 Spam 12", "2"),
                ctl("3 Promotions 41", "3"),
                ctl("4 Notifications 88", "4"),
                ctl("5 Receipts 19", "5"),
                ctl("6 Shipping 14", "6"),
                ctl("7 Social 12", "7"),
                ctl("Promo Weekly", "j"),
                ctl("↵ open", "<Enter>"),
            ],
            holds: false,
            ..case("Filtered", "filtered", (120, 36))
        },
        Case {
            then: "<Enter>",
            marks: vec![Has("╭"), Styled("Filtered 12", BOLD)],
            controls: vec![
                ctl("↓ j", "j"),
                ctl("Esc ✕", "<Esc>"),
                ctl("Archive a", "a"),
                ctl("Snooze s", "s"),
                ctl("Reply e", "e"),
            ],
            holds: false,
            ..case("a filtered message open for itself", "filtered", (120, 36))
        },
        Case {
            wheel: Some("Forge"),
            holds: false,
            ..case("Filtered, scrolling", "filtered", (120, 12))
        },
        Case {
            marks: vec![
                Has("╭"),
                Has("Cancel Esc"),
                Styled("Move 7 to Filtered", BOLD),
            ],
            controls: vec![
                ctl("[ Cancel Esc ]", "<Esc>"),
                ctl("[ Move 7 to Filtered ↵ ]", "<Enter>"),
            ],
            ..case("sweep question", "sweep", (120, 36))
        },
        Case {
            marks: vec![Has("≡"), Has("[1]"), Styled("[1]", REVERSED)],
            controls: vec![
                ctl("Archive all 14 A", "A"),
                ctl("14 messages  tab", "<Tab>"),
                ctl("Edit rule and cadence d", "d"),
                ctl("The 8:10 train moves", "]"),
                ctl("↵ open the full email", "<Enter>"),
                ctl("] [ next / previous reference", "]"),
                ctl("↵ open", "<Enter>").nth(1),
                ctl("tab summary / messages", "<Tab>"),
                ctl("D stop digesting the sender", "D"),
                ctl("Esc ✕", "<Esc>"),
            ],
            holds: false,
            ..case("digest summary", "digest", (120, 36))
        },
        Case {
            wheel: Some("dredging"),
            holds: false,
            ..case("digest summary, scrolling", "digest", (120, 16))
        },
        Case {
            marks: vec![Has("▌")],
            controls: vec![
                ctl("Archive all 14 A", "A"),
                ctl("Summary", "<Tab>"),
                ctl("Rail Notes", "j"),
                ctl("↵ open", "<Enter>").nth(0),
                ctl("tab summary / messages", "<Tab>"),
                ctl("D stop digesting the sender", "D"),
                ctl("U unsubscribe", "U"),
            ],
            holds: false,
            ..case("digest list", "digest-list", (120, 36))
        },
        Case {
            marks: vec![Has("‹ Summary")],
            controls: vec![
                ctl("‹ Summary Esc", "<Esc>"),
                ctl("j k next / previous source", "j"),
                ctl("D stop digesting the sender", "D"),
                ctl("U unsubscribe", "U"),
            ],
            holds: false,
            ..case("digest email", "digest-email", (120, 36))
        },
        Case {
            marks: vec![Has("▌")],
            controls: vec![
                ctl("‹ Inbox g i", "gi"),
                ctl("↵ edit", "<Enter>"),
                ctl("Del remove and release", "<Del>"),
                ctl("Esc inbox", "<Esc>"),
                ctl("Receipts", "j"),
            ],
            holds: false,
            ..case("digest rules", "rules", (120, 36))
        },
        Case {
            marks: vec![
                Has("╭"),
                Has("Cancel Esc"),
                Styled("Save", BOLD),
                Styled("Weekly ▾", REVERSED),
            ],
            controls: vec![ctl("[ Cancel Esc ]", "<Esc>"), ctl("[ Save ↵ ]", "<Enter>")],
            ..case("rule dialog", "rule", (120, 36))
        },
        Case {
            marks: vec![
                Has("╭"),
                StyledAt("Task t", 1, REVERSED),
                Styled("Add task", BOLD),
            ],
            controls: vec![
                ctl("Note n", "n").after("<Tab>"),
                ctl(" Today ", "<Right>").nth(1).after("<Tab>"),
                ctl(" Fri ", "<Left>").nth(1).after("<Tab><Right>"),
                ctl("use the subject instead alt+s", "<A-s>"),
                ctl("Change ctrl+p", "<C-p>"),
                ctl("[ Add task alt+↵ ]", "<A-Enter>"),
                ctl("[ Cancel Esc ]", "<Esc>"),
            ],
            ..case("capture", "capture", (120, 36))
        },
    ]
}

// -- Reading a screen ------------------------------------------------------

/// The text of row `y` of `buffer`, one character per cell.
fn row_text(buffer: &Buffer, y: u16) -> Vec<String> {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_owned())
        .collect()
}

/// Where the `nth` occurrence of `text` starts, reading the screen from the
/// top left: its column and row.
fn find(buffer: &Buffer, text: &str, nth: usize) -> Option<(u16, u16)> {
    let wanted: Vec<String> = text.chars().map(String::from).collect();
    let mut seen = 0;
    for y in 0..buffer.area.height {
        let cells = row_text(buffer, y);
        for x in 0..cells.len().saturating_sub(wanted.len() - 1) {
            if cells[x..x + wanted.len()] == wanted[..] {
                if seen == nth {
                    return Some((x as u16, y));
                }
                seen += 1;
            }
        }
    }
    None
}

fn modifiers_of(buffer: &Buffer, text: &str) -> Vec<Modifier> {
    let (x, y) = find(buffer, text, 0).expect("drawn");
    (0..text.chars().count() as u16)
        .map(|at| buffer[(x + at, y)].modifier)
        .collect()
}

fn app_of(case: &Case) -> App {
    let (mut app, _) = sample::state(case.state, case.size.0, case.size.1);
    press_keys(&mut app, case.then);
    app
}

// -- The sweep -------------------------------------------------------------

#[test]
fn every_surface_keeps_its_marks_without_colour() {
    let mut problems = Vec::new();
    for case in cases() {
        let app = app_of(&case);
        let drawn = buffer(case.size.0, case.size.1, &app);
        let text = screen(case.size.0, case.size.1, &app);
        for mark in &case.marks {
            let problem = match mark {
                Mark::Has(wanted) => {
                    (!text.contains(wanted)).then(|| format!("{wanted:?} is missing"))
                }
                Mark::StyledAt(wanted, nth, modifier) => match find(&drawn, wanted, *nth) {
                    None => Some(format!("{wanted:?} is not drawn")),
                    Some((x, y)) => (!(0..wanted.chars().count() as u16)
                        .all(|at| drawn[(x + at, y)].modifier.contains(*modifier)))
                    .then(|| format!("{wanted:?} is not {modifier:?}")),
                },
                Mark::Styled(wanted, modifier) | Mark::Plain(wanted, modifier) => {
                    let had = find(&drawn, wanted, 0).map(|_| modifiers_of(&drawn, wanted));
                    let styled = matches!(mark, Mark::Styled(..));
                    match had {
                        None => Some(format!("{wanted:?} is not drawn")),
                        Some(had) if had.iter().all(|had| had.contains(*modifier) == styled) => {
                            None
                        }
                        Some(_) => Some(format!(
                            "{wanted:?} is {}{modifier:?}",
                            if styled { "not " } else { "" }
                        )),
                    }
                }
            };
            problems.extend(problem.map(|problem| format!("{}: {problem}", case.surface)));
        }
    }
    assert!(
        problems.is_empty(),
        "under NO_COLOR:\n{}",
        problems.join("\n")
    );
}

/// What was sent, with the clock's readings taken out, and what a person sees.
#[derive(PartialEq)]
struct Outcome {
    sent: String,
    screen: String,
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sent {}", self.sent)
    }
}

/// `text` with every instant (`2026-07-08T23:11:04.77Z`) replaced.
fn without_instants(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut at = 0;
    while at < bytes.len() {
        let is_instant = bytes.len() >= at + 20
            && bytes[at..at + 4].iter().all(u8::is_ascii_digit)
            && bytes[at + 4] == b'-'
            && bytes[at + 10] == b'T';
        if is_instant {
            let end = text[at..].find('Z').map_or(bytes.len(), |end| at + end + 1);
            out.push_str("<instant>");
            at = end;
        } else {
            let c = text[at..].chars().next().expect("a character");
            out.push(c);
            at += c.len_utf8();
        }
    }
    out
}

/// What happens after `prep` and then `act`, in a fresh app.
fn outcome(case: &Case, prep: &str, act: impl FnOnce(&mut App) -> Vec<Effect>) -> Outcome {
    let mut app = app_of(case);
    press_keys(&mut app, prep);
    let effects = act(&mut app);
    // A redraw is the loop drawing, which the screen says.
    let sent: Vec<&Effect> = effects
        .iter()
        .filter(|effect| **effect != Effect::Redraw)
        .collect();
    Outcome {
        sent: without_instants(&format!("{sent:?}")),
        screen: screen(case.size.0, case.size.1, &app),
    }
}

/// What nothing happening looks like after `prep`.
fn unchanged(case: &Case, prep: &str) -> Outcome {
    outcome(case, prep, |_| Vec::new())
}

#[test]
fn every_control_takes_a_click_that_does_what_its_key_does() {
    let mut problems = Vec::new();
    for case in cases() {
        for control in &case.controls {
            let mut app = app_of(&case);
            press_keys(&mut app, control.prep);
            let drawn = buffer(case.size.0, case.size.1, &app);
            let Some(at) = find(&drawn, control.label, control.nth) else {
                problems.push(format!(
                    "{}: {:?} is not drawn",
                    case.surface, control.label
                ));
                continue;
            };
            let clicked = outcome(&case, control.prep, |app| click_at(app, case.size, at));
            // A control with no key is its own: it must at least do something.
            if control.keys.is_empty() {
                if clicked == unchanged(&case, control.prep) {
                    problems.push(format!(
                        "{}: {:?} does nothing",
                        case.surface, control.label
                    ));
                }
                continue;
            }
            let keyed = outcome(&case, control.prep, |app| press_keys(app, control.keys));
            if clicked.sent != keyed.sent {
                problems.push(format!(
                    "{}: a click on {:?} sends {} where {:?} sends {}",
                    case.surface, control.label, clicked.sent, control.keys, keyed.sent
                ));
            } else if clicked.screen != keyed.screen {
                let differing: Vec<String> = clicked
                    .screen
                    .lines()
                    .zip(keyed.screen.lines())
                    .filter(|(click, key)| click != key)
                    .map(|(click, key)| {
                        format!("  click: {}\n  key:   {}", click.trim_end(), key.trim_end())
                    })
                    .collect();
                problems.push(format!(
                    "{}: a click on {:?} draws other than {:?} does\n{}",
                    case.surface,
                    control.label,
                    control.keys,
                    differing.join("\n")
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// What a region is, apart from where in its list it is.
fn kind(target: Target) -> String {
    match target {
        Target::RowAction(_, id) => format!("RowAction {id}"),
        Target::Surface(part, _) => format!("Surface {part:?}"),
        Target::BarSaved(_) => "BarSaved".into(),
        Target::BarChip(_) => "BarChip".into(),
        other => format!("{other:?}"),
    }
}

#[test]
fn every_control_a_surface_draws_is_covered_by_a_case() {
    let at_of = |case: &Case, control: &Control| {
        let mut app = app_of(case);
        press_keys(&mut app, control.prep);
        let hits = hits_of(case.size.0, case.size.1, &app);
        let drawn = buffer(case.size.0, case.size.1, &app);
        find(&drawn, control.label, control.nth)
            .and_then(|(x, y)| hits.at(x, y))
            .map(|hit| kind(hit.target))
    };
    let all = cases();
    let covered: Vec<String> = all
        .iter()
        .flat_map(|case| {
            case.controls
                .iter()
                .filter_map(|control| at_of(case, control))
        })
        .collect();
    let mut missing = Vec::new();
    for case in &all {
        let app = app_of(case);
        let hits = hits_of(case.size.0, case.size.1, &app);
        let drawn = buffer(case.size.0, case.size.1, &app);
        for (area, target) in hits.regions() {
            if is_row_like(*target) || covered.contains(&kind(*target)) {
                continue;
            }
            // A region under a frame that holds the pointer is drawn, and
            // heard by nothing.
            let live = outcome(case, "", |app| click_at(app, case.size, (area.x, area.y)));
            if live == unchanged(case, "") {
                continue;
            }
            let text: String = (area.x..area.x + area.width.min(40))
                .map(|x| drawn[(x, area.y)].symbol().to_owned())
                .collect();
            missing.push(format!("{}: {} at {text:?}", case.surface, kind(*target)));
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "controls no case clicks:\n{}",
        missing.join("\n")
    );
}

/// A region of a list, or of text to point into, rather than a control.
fn is_row_like(target: Target) -> bool {
    use crate::surface::Part;
    matches!(
        target,
        Target::Row(_)
            | Target::Reader(_)
            | Target::ComposerBody
            | Target::ComposerField(_)
            | Target::BarRow(_)
            | Target::PlaceRow(_)
            | Target::PickRow(_)
            | Target::MenuRow(_)
            | Target::Overlay
            | Target::Surface(
                Part::FilteredRow
                    | Part::DigestRow
                    | Part::RuleRow
                    | Part::RuleField
                    | Part::CaptureText,
                _
            )
    )
}

#[test]
fn the_wheel_scrolls_what_scrolls() {
    let mut problems = Vec::new();
    for case in cases() {
        let Some(over) = case.wheel else { continue };
        let app = app_of(&case);
        let drawn = buffer(case.size.0, case.size.1, &app);
        let Some(at) = find(&drawn, over, 0) else {
            problems.push(format!("{}: {over:?} is not drawn", case.surface));
            continue;
        };
        let before = screen(case.size.0, case.size.1, &app);
        // One way or the other: a view at its end scrolls only back.
        let mut heard = false;
        let mut moved = false;
        for down in [true, false] {
            let mut app = app_of(&case);
            heard |= !wheel_at(&mut app, case.size, at, down).is_empty();
            moved |= before != screen(case.size.0, case.size.1, &app);
        }
        if !heard {
            problems.push(format!(
                "{}: the wheel was not heard over {over:?}",
                case.surface
            ));
        } else if !moved {
            problems.push(format!(
                "{}: the wheel scrolled nothing over {over:?}",
                case.surface
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn with_the_mouse_off_no_pointer_input_does_anything_and_every_key_still_does() {
    for case in cases() {
        let (width, height) = case.size;
        let mut app = app_of(&case).with_mouse(false);
        let before = screen(width, height, &app);
        let hits = hits_of(width, height, &app);
        for (area, _) in hits.regions() {
            let at = (area.x, area.y);
            let on = |app: &mut App, kind| mouse_on(app, &hits, at, kind, KeyModifiers::NONE);
            assert!(
                on(&mut app, MouseEventKind::Down(MouseButton::Left)).is_empty()
                    && on(&mut app, MouseEventKind::ScrollDown).is_empty(),
                "{}: the mouse acted at {at:?} with mouse = false",
                case.surface
            );
        }
        assert_eq!(before, screen(width, height, &app), "{}", case.surface);
        for control in case
            .controls
            .iter()
            .filter(|control| !control.keys.is_empty())
        {
            let keyed = outcome(&case, control.prep, |app| press_keys(app, control.keys));
            assert!(
                keyed != unchanged(&case, control.prep),
                "{}: {:?} by key does nothing",
                case.surface,
                control.label
            );
        }
    }
}

#[test]
fn a_click_outside_a_frame_does_nothing() {
    let mut problems = Vec::new();
    for case in cases() {
        let (width, height) = case.size;
        let app = app_of(&case);
        let hits = hits_of(width, height, &app);
        let regions = hits.regions();
        if !case.modal {
            continue;
        }
        let Some(frame) = regions
            .iter()
            .rposition(|(_, target)| *target == Target::Overlay)
        else {
            continue;
        };
        for (area, target) in &regions[..frame] {
            // A cell where this region is what a click would land on.
            let cell = (area.y..area.y + area.height)
                .flat_map(|y| (area.x..area.x + area.width).map(move |x| (x, y)))
                .find(|&(x, y)| hits.at(x, y).is_some_and(|hit| hit.target == *target));
            let Some((x, y)) = cell else { continue };
            if *target == Target::Overlay || (!case.holds && (y == 0 || y == height - 1)) {
                continue;
            }
            let clicked = outcome(&case, "", |app| click_at(app, case.size, (x, y)));
            if clicked != unchanged(&case, "") {
                problems.push(format!(
                    "{}: a click at {x},{y} on {target:?} does something",
                    case.surface
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// `ctrl+q` quits from wherever the person is: the one key that has to work
/// on every surface, text fields and frames included.
#[test]
fn ctrl_q_quits_from_every_surface() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut stuck = Vec::new();
    for state in std::iter::once("list").chain(crate::test_support::sample::STATES.iter().copied())
    {
        let (mut app, _) = crate::test_support::sample::state(state, 120, 36);
        let effects = crate::app::update(
            &mut app,
            crate::app::Input::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        );
        if !effects.contains(&crate::app::Effect::Quit) {
            stuck.push(format!("{state}: {effects:?}"));
        }
    }
    assert!(
        stuck.is_empty(),
        "ctrl+q did not quit from:\n{}",
        stuck.join("\n")
    );
}

/// Quit's other key, `ctrl+w`, is a text field's "delete the word before",
/// so in the command bar it edits what was typed and quits nothing.
#[test]
fn ctrl_w_in_the_bar_edits_and_does_not_quit() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, _) = crate::test_support::sample::state("bar", 120, 36);
    let effects = crate::app::update(
        &mut app,
        crate::app::Input::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
    );
    assert!(!effects.contains(&crate::app::Effect::Quit), "{effects:?}");
}
