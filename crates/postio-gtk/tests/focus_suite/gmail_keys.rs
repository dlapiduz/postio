//! Gmail's keyboard shortcuts, pressed end to end in the real window
//! (specs/007-postio-focus contracts/keymap.md, the 2026-10-14 amendment;
//! docs/PRODUCT.md §8).
//!
//! The registry says which key runs which command. These cases say the
//! window agrees, for every command with a Gmail key and every surface that
//! offers it: the key is spelled as a US keyboard delivers it (a capital or
//! a shifted symbol arrives as its own keyval with Shift held, `#` as
//! `numbersign`, `*` as `asterisk`, `;` bare), pressed through GTK's own
//! controller chain, and judged by what a person would see -- the cursor,
//! the rows, the message in the store, the dialog that opened -- never by
//! what a layer was handed.
//!
//! The table is the registry's. A command's bindings (its default and its
//! alternates) and its contexts come from `postio_core::registry`; the cases
//! only say what each command does where, so a binding the registry gains
//! is pressed without anyone remembering to add it here, and a command with
//! a Gmail key that no probe or reason accounts for fails
//! `every_gmail_command_is_probed_or_excused`.

use std::cell::RefCell;
use std::fmt::Write as _;

use gtk::gdk::ModifierType as Mods;
use gtk::prelude::*;
use postio_core::{ActionId, CommandId, Context, Keymap, registry};
use postio_model::{Flag, MailboxId, MailboxRole, MessageId};
use postio_storage::repository::{
    MailboxRepository, MessageRepository, StoredBody, UnsubscribeRepository,
};
use postio_ui::keymap::{Chord, Key, Modifiers};
use postio_ui::observe::Observation;

use crate::support::{self, Fixture};

type Window = postio_gtk::window::FocusWindow;

// ---------------------------------------------------------------------------
// The keyboard
// ---------------------------------------------------------------------------

/// What a US keyboard types with Shift held, and the keysym GTK names it:
/// the key's keyval is the symbol, and Shift is in the state.
const US_SHIFTED: &[(char, &str)] = &[
    ('!', "exclam"),
    ('@', "at"),
    ('#', "numbersign"),
    ('$', "dollar"),
    ('%', "percent"),
    ('^', "asciicircum"),
    ('&', "ampersand"),
    ('*', "asterisk"),
    ('(', "parenleft"),
    (')', "parenright"),
    ('_', "underscore"),
    ('+', "plus"),
    ('{', "braceleft"),
    ('}', "braceright"),
    ('|', "bar"),
    (':', "colon"),
    ('"', "quotedbl"),
    ('<', "less"),
    ('>', "greater"),
    ('?', "question"),
    ('~', "asciitilde"),
];

/// What a US keyboard types without it, for the punctuation a binding uses.
const US_PLAIN: &[(char, &str)] = &[
    (';', "semicolon"),
    (',', "comma"),
    ('.', "period"),
    ('/', "slash"),
    ('-', "minus"),
    ('=', "equal"),
    ('[', "bracketleft"),
    (']', "bracketright"),
    ('\\', "backslash"),
    ('\'', "apostrophe"),
    ('`', "grave"),
];

/// The key a person presses for `chord` on a US keyboard: the keysym GTK
/// delivers and the modifiers held. Written out here rather than asked of
/// the code under test, which is also what the storyboard runner uses.
fn us_keyboard(chord: &Chord) -> (String, Mods) {
    let mut state = Mods::empty();
    for (modifier, mask) in [
        (Modifiers::CTRL, Mods::CONTROL_MASK),
        (Modifiers::ALT, Mods::ALT_MASK),
        (Modifiers::SHIFT, Mods::SHIFT_MASK),
        (Modifiers::SUPER, Mods::SUPER_MASK),
    ] {
        if chord.modifiers.contains(modifier) {
            state |= mask;
        }
    }
    let name = match &chord.key {
        Key::Char(c) if c.is_ascii_uppercase() => {
            state |= Mods::SHIFT_MASK;
            c.to_string()
        }
        Key::Char(c) => {
            if let Some((_, name)) = US_SHIFTED.iter().find(|(shifted, _)| shifted == c) {
                state |= Mods::SHIFT_MASK;
                (*name).to_owned()
            } else if let Some((_, name)) = US_PLAIN.iter().find(|(plain, _)| plain == c) {
                (*name).to_owned()
            } else {
                c.to_string()
            }
        }
        Key::Named("Space") => "space".to_owned(),
        Key::Named(name) => (*name).to_owned(),
    };
    (name, state)
}

/// Press every chord of `binding` (`"g i"`, `"ctrl+shift+a"`, `"#"`), one
/// after another, as the keyboard delivers them. Whether the last was taken.
fn press_binding(window: &Window, binding: &str) -> bool {
    let mut taken = false;
    for chord in binding.split(' ') {
        let chord: Chord = chord
            .parse()
            .unwrap_or_else(|_| panic!("{binding:?} is not a key the registry can spell"));
        let (name, state) = us_keyboard(&chord);
        taken = support::deliver_with(window, &name, state);
    }
    taken
}

/// Every key that runs `command` by default: its primary and its
/// alternates, as this machine spells them.
fn bindings_of(command: CommandId) -> Vec<String> {
    Keymap::defaults()
        .bindings(ActionId::Builtin(command))
        .to_vec()
}

// ---------------------------------------------------------------------------
// The surfaces
// ---------------------------------------------------------------------------

/// Where the keyboard is when the key is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    /// The inbox list, the cursor on its first row.
    List,
    /// The message open in its dialog.
    Reader,
    /// The message open beside the list (F8).
    Pane,
    /// A digest's window, over the inbox.
    Digest,
    /// The Filtered view.
    Filtered,
    /// The capture sheet.
    Capture,
    /// The menu a right-click opens over a row, which runs a verb's key.
    Menu,
}

impl Surface {
    const ALL: [Surface; 7] = [
        Surface::List,
        Surface::Reader,
        Surface::Pane,
        Surface::Digest,
        Surface::Filtered,
        Surface::Capture,
        Surface::Menu,
    ];

    /// The registry's context this surface is.
    fn context(self) -> Context {
        match self {
            Surface::List | Surface::Menu => Context::List,
            Surface::Reader | Surface::Pane => Context::Reader,
            Surface::Digest => Context::Digest,
            Surface::Filtered => Context::Filtered,
            Surface::Capture => Context::Capture,
        }
    }
}

/// What the mailbox holds, because some keys need a particular message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    /// Twenty plain conversations.
    Plain,
    /// The first row has an attached file.
    Attachment,
    /// The first row is a newsletter, offering to unsubscribe.
    Newsletter,
    /// The first row is a conversation of three messages.
    Thread,
    /// Plain, over a vault the capture sheet can write to.
    Vault,
    /// Three conversations and a digest of two newsletters.
    Digest,
    /// One conversation, and three filtered messages.
    Filtered,
}

/// A mailbox, a window over it, and what is needed to read either.
struct World {
    fixture: Fixture,
    window: Window,
    /// What a digest holds, newest first.
    held: Vec<MessageId>,
    /// The conversation of the thread flavor, oldest first.
    thread: Vec<MessageId>,
    /// The mailboxes the verbs move to.
    archive: Option<MailboxId>,
    trash: Option<MailboxId>,
    /// Keeps the vault's folder alive.
    _vault: Option<tempfile::TempDir>,
}

const NEWSLETTER: &[u8] = b"From: Weekly Digest <weekly@news.example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: This week in Postio\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Nothing much happened\r\n";

const RAW: &str =
    "From: Ada Moreno <ada@example.com>\r\nTo: you@example.com\r\nSubject: @@\r\n\r\nThe body.\r\n";

impl World {
    async fn new(flavor: Flavor, surface: Surface) -> World {
        let mut held = Vec::new();
        let fixture = match flavor {
            Flavor::Digest => {
                let (fixture, _, messages) = crate::digest::delivered_holding().await;
                held = messages;
                fixture
            }
            _ => Fixture::empty().await,
        };
        let mut first = 0;
        let mut thread = Vec::new();
        match flavor {
            Flavor::Attachment => {
                let id = fixture
                    .file_with_attachment("Has a file", "report.pdf")
                    .await;
                fixture.write_body(id, "See the attached file.").await;
                first = 1;
            }
            Flavor::Newsletter => {
                let connection = fixture.database.connect().await.expect("a connection");
                let parsed = postio_model::mime::parse(NEWSLETTER);
                let body = parsed.body.clone();
                let problems = parsed.encoding_problems;
                let mut message = parsed.into_message(
                    fixture.account.id,
                    fixture.inbox,
                    support::now() - chrono::Duration::minutes(5),
                );
                message.sync.body_state = postio_model::BodyState::Full;
                message.promoted = Some(postio_model::promoted::PromotedHeaders {
                    unsubscribe_offered: true,
                    automation: 0,
                });
                let id = MessageRepository::new(&connection)
                    .create(&mut message)
                    .await
                    .expect("a message");
                MessageRepository::new(&connection)
                    .set_body(
                        id,
                        &StoredBody {
                            text: body.text,
                            html: body.html,
                            headers: None,
                            headers_truncated: false,
                            encoding_problems: problems,
                        },
                        postio_model::BodyState::Full,
                    )
                    .await
                    .expect("a body");
                first = 1;
            }
            Flavor::Thread => {
                thread = fixture.thread_of("Plans", 3, 5).await;
                first = 1;
            }
            Flavor::Filtered => {
                fixture
                    .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 5)
                    .await;
                for (from, subject, reason, minutes) in [
                    (
                        ("Forge", "noreply@forge.test"),
                        "Review requested",
                        "notification",
                        10,
                    ),
                    (
                        ("Outdoor Supply", "deals@outdoor.test"),
                        "30% off tents",
                        "promotion",
                        20,
                    ),
                    (
                        ("Lucky Rewards", "win@lucky.test"),
                        "You have been selected",
                        "spam",
                        30,
                    ),
                ] {
                    fixture
                        .filtered(from, subject, reason, Some("Forge"), minutes)
                        .await;
                }
            }
            Flavor::Digest | Flavor::Plain | Flavor::Vault => {}
        }
        if !matches!(flavor, Flavor::Digest | Flavor::Filtered) {
            for n in first..20 {
                let subject = format!("M{:02}", n + 1);
                let (id, _) = fixture
                    .file(
                        ("Ada Moreno", "ada@example.com"),
                        &subject,
                        "x",
                        (n as i64 + 1) * 10 + 20,
                    )
                    .await;
                fixture.write_body(id, &format!("{subject} body.")).await;
                fixture
                    .write_raw(id, RAW.replace("@@", &subject).as_bytes())
                    .await;
            }
        }
        for folder in ["Drafts", "Sent", "Junk"] {
            fixture.folder(folder).await;
        }
        let (config, vault) = if flavor == Flavor::Vault || surface == Surface::Capture {
            let dir = tempfile::tempdir().expect("a vault folder");
            std::fs::create_dir(dir.path().join("Projects")).expect("projects");
            std::fs::write(dir.path().join("Tasks.md"), "# Tasks\n").expect("tasks");
            let config = postio_config::Config::from_toml_str(&format!(
                "[focus.vault]\npath = \"{}\"\nprojects = \"Projects\"\n",
                dir.path().display()
            ))
            .expect("a config with a vault");
            (config, Some(dir))
        } else {
            (postio_config::Config::default(), None)
        };
        let window = Window::new(None);
        window.present();
        let session = postio_gtk::startup::adopt(&window, fixture.host(), &config);
        support::keep(session);
        let wanted = match flavor {
            Flavor::Digest => 4,
            Flavor::Filtered => 1,
            _ => 8,
        };
        assert!(
            crate::settle_until(async || window.rows_on_screen().len() >= wanted).await,
            "the mailbox never reached the screen: {:?}",
            window.rows_on_screen()
        );
        if surface == Surface::Pane {
            window.act(CommandId::ToggleReadingPane);
            crate::settle();
        }
        let connection = fixture.database.connect().await.expect("a connection");
        let role = async |role| {
            MailboxRepository::new(&connection)
                .by_role(fixture.account.id, role)
                .await
                .expect("a read")
                .map(|mailbox| mailbox.id)
        };
        let archive = role(MailboxRole::Archive).await;
        let trash = role(MailboxRole::Trash).await;
        drop(connection);
        World {
            fixture,
            window,
            held,
            thread,
            archive,
            trash,
            _vault: vault,
        }
    }

    /// Put the window back as a person finds it: nothing open, the inbox,
    /// the cursor on its first row, the keyboard on the list. Escape does
    /// the closing, as a person's would.
    async fn reset(&self) {
        let window = &self.window;
        // A message that is still drawing is let finish: closing it under
        // its web view mid-paint is not what a person's Escape does.
        if let Some(reading) = window.reading().filter(|reading| reading.is_open()) {
            crate::settle_until_within(std::time::Duration::from_millis(1500), async || {
                reading.reader().view().tiles_settled()
            })
            .await;
        }
        for _ in 0..8 {
            let open = window.open_picker().is_some()
                || window.key_map().is_some()
                || window.reading().is_some_and(|reading| reading.is_open())
                || window.capture().is_some_and(|sheet| sheet.is_shown())
                || window.digest().is_some()
                || window.bar().is_some_and(|bar| bar.is_open())
                || window.places().is_some_and(|places| places.is_open())
                || window.row_menu().is_some_and(|menu| menu.is_open())
                || window.source_shown().is_some()
                || !window.choices_shown().is_empty()
                || window.observe().composer.open
                || window.observe().selection.count > 0;
            if !open {
                break;
            }
            support::deliver_with(window, "Escape", Mods::empty());
        }
        window.act(CommandId::GoToInbox);
        crate::settle_until(async || {
            window.place_name() == "Inbox"
                && window.rules().is_none()
                && window.filtered().is_none()
                && window.pane().is_some_and(|pane| pane.feed().has_landed())
        })
        .await;
        window.focus_list();
        window.act(CommandId::FirstMessage);
        crate::settle_until(async || window.observe().cursor.index == Some(0)).await;
        window.take_unanswered();
    }

    /// Wait until the open message is drawn with its own words. The reader
    /// shows a plate while it reads a message and keeps drawing the one
    /// before until the new snapshot arrives, so what is on screen right
    /// after a step is not yet what the step went to.
    async fn drawn(&self) -> bool {
        let window = &self.window;
        crate::settle_until(async || {
            let Some(reading) = window.reading().filter(|reading| reading.is_open()) else {
                return false;
            };
            let Some(at) = reading
                .shown()
                .and_then(|shown| self.thread.iter().position(|id| *id == shown))
            else {
                return false;
            };
            reading.body_text() == format!("Message {}", at + 1)
        })
        .await
    }

    /// The message the cursor is on.
    fn aimed(&self) -> Option<MessageId> {
        self.window.cursor_row().map(|row| row.id())
    }

    async fn stored(&self, message: MessageId) -> Stored {
        let connection = self.fixture.database.connect().await.expect("a connection");
        let stored = MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        Stored {
            flags: stored.flags.iter().cloned().collect(),
            mailbox: stored.mailbox_id,
        }
    }

    async fn unsubscribed(&self) -> usize {
        let connection = self.fixture.database.connect().await.expect("a connection");
        UnsubscribeRepository::new(&connection)
            .for_account(self.fixture.account.id)
            .await
            .expect("the log")
            .len()
    }

    /// Bring `surface` up over the cursor's row.
    async fn enter(&self, surface: Surface) {
        let window = &self.window;
        match surface {
            Surface::List => {}
            Surface::Reader | Surface::Pane => {
                window.act(CommandId::OpenMessage);
                let opened = crate::settle_until(async || {
                    window
                        .reading()
                        .is_some_and(|reading| reading.is_open() && !reading.body_text().is_empty())
                })
                .await;
                assert!(opened, "{surface:?}: the message never opened");
                if let Some(reading) = window.reading() {
                    crate::settle_until(async || reading.reader().view().tiles_settled()).await;
                }
            }
            Surface::Digest => {
                // The digest's row is wherever it sorts; the cursor walks to it.
                for _ in 0..8 {
                    if matches!(
                        window.cursor_row(),
                        Some(postio_gtk::list::FocusRow::Digest(_))
                    ) {
                        break;
                    }
                    window.act(CommandId::NextMessage);
                    crate::settle();
                }
                window.act(CommandId::OpenMessage);
                let digest = crate::settle_until(async || {
                    window
                        .digest()
                        .is_some_and(|digest| digest.subjects().len() == 2)
                })
                .await;
                assert!(digest, "the digest never opened over its row");
            }
            Surface::Filtered => {
                window.act(CommandId::GoToFiltered);
                let shown = crate::settle_until(async || {
                    window
                        .filtered()
                        .is_some_and(|view| view.subjects().len() == 3)
                })
                .await;
                assert!(shown, "Filtered never listed its three messages");
            }
            Surface::Menu => {
                window.open_row_menu(0, gtk::gdk::Rectangle::new(40, 120, 1, 1));
                let open = crate::settle_until(async || {
                    window.row_menu().is_some_and(|menu| menu.is_open())
                })
                .await;
                assert!(open, "the row's menu never opened");
            }
            Surface::Capture => {
                window.act(CommandId::CaptureNote);
                let shown = crate::settle_until(async || {
                    window.capture().is_some_and(|sheet| sheet.is_shown())
                })
                .await;
                assert!(shown, "the capture sheet never opened");
            }
        }
    }
}

/// A message as the store holds it.
#[derive(Clone, Debug, PartialEq)]
struct Stored {
    flags: Vec<Flag>,
    mailbox: MailboxId,
}

/// Everything a person could tell about the window at a moment.
#[derive(Clone, Debug)]
struct Snap {
    obs: Observation,
    subjects: Vec<String>,
    toast: Option<String>,
    place: String,
    reading_open: bool,
    reading_title: String,
    reading_body: String,
    more: bool,
    source: bool,
    choices: usize,
    capture: Option<Vec<String>>,
    compose: bool,
    places: bool,
    rules: bool,
    filtered: bool,
    filtered_focus: Option<MessageId>,
    digest: bool,
    digest_focus: Option<MessageId>,
    key_map: bool,
    bar: bool,
    picker: Option<String>,
    stored: Option<Stored>,
    unsubscribed: usize,
}

impl Snap {
    async fn take(world: &World, watch: Option<MessageId>) -> Snap {
        let window = &world.window;
        let reading = window.reading().filter(|reading| reading.is_open());
        Snap {
            obs: window.observe(),
            subjects: support::subjects(window),
            toast: window.toast_showing(),
            place: window.place_name(),
            reading_open: reading.is_some(),
            reading_title: reading
                .as_ref()
                .map(|reading| reading.title())
                .unwrap_or_default(),
            reading_body: reading
                .as_ref()
                .map(|reading| reading.body_text())
                .unwrap_or_default(),
            more: reading.as_ref().is_some_and(|reading| reading.more_open()),
            source: window.source_shown().is_some(),
            choices: window.choices_shown().len(),
            capture: window
                .capture()
                .filter(|sheet| sheet.is_shown())
                .map(|sheet| sheet.texts()),
            compose: window.observe().composer.open,
            places: window.places().is_some_and(|places| places.is_open()),
            rules: window.rules().is_some(),
            filtered: window.filtered().is_some(),
            filtered_focus: window.filtered().and_then(|view| view.focused()),
            digest: window.digest().is_some(),
            digest_focus: window
                .digest()
                .and_then(|digest| digest.focused())
                .map(|message| message.id),
            key_map: window.key_map().is_some(),
            bar: window.bar().is_some_and(|bar| bar.is_open()),
            picker: window
                .open_picker()
                .map(|picker| picker.texts().join(" ").to_lowercase()),
            stored: match watch {
                Some(message) => Some(world.stored(message).await),
                None => None,
            },
            unsubscribed: world.unsubscribed().await,
        }
    }

    fn cursor(&self) -> Option<u32> {
        self.obs.cursor.index
    }

    fn selected(&self) -> u32 {
        self.obs.selection.count
    }

    fn has(&self, flag: Flag) -> bool {
        self.stored
            .as_ref()
            .is_some_and(|stored| stored.flags.contains(&flag))
    }

    fn mailbox(&self) -> Option<MailboxId> {
        self.stored.as_ref().map(|stored| stored.mailbox)
    }
}

/// What a probe is given to judge.
struct Seen<'a> {
    before: &'a Snap,
    after: &'a Snap,
    archive: Option<MailboxId>,
    trash: Option<MailboxId>,
    /// Commands the window had no arm for since the key was pressed.
    unanswered: &'a [CommandId],
    /// How many rows the mailbox had on screen.
    rows: usize,
}

type Verdict = fn(&Seen) -> Result<(), String>;

/// What to do before the key, and what the key must have done.
struct Probe {
    flavor: Flavor,
    /// Rows to step the cursor down before the surface comes up.
    cursor_down: usize,
    /// A command run (not pressed) before the key, whose effect the key is
    /// then judged against. For the list and the open message it runs
    /// before the surface comes up; for the others, once it is up.
    before: Option<CommandId>,
    /// Something done once the surface is up, before the key.
    once_up: Option<fn(&Window)>,
    /// What a person would see, in a sentence.
    expected: &'static str,
    verdict: Verdict,
    /// The key undoes what the world is made of (a digest, a filtered
    /// message), so the next press gets a world of its own.
    fresh: bool,
}

impl Probe {
    fn new(expected: &'static str, verdict: Verdict) -> Probe {
        Probe {
            flavor: Flavor::Plain,
            cursor_down: 0,
            before: None,
            once_up: None,
            expected,
            verdict,
            fresh: false,
        }
    }

    fn fresh(mut self) -> Probe {
        self.fresh = true;
        self
    }

    fn on(mut self, flavor: Flavor) -> Probe {
        self.flavor = flavor;
        self
    }

    fn down(mut self, rows: usize) -> Probe {
        self.cursor_down = rows;
        self
    }

    fn after(mut self, command: CommandId) -> Probe {
        self.before = Some(command);
        self
    }

    fn once_up(mut self, step: fn(&Window)) -> Probe {
        self.once_up = Some(step);
        self
    }
}

fn said(seen: &Seen) -> String {
    let after = seen.after;
    let mut line = format!(
        "cursor {:?}, selected {}, rows {}, place {:?}",
        after.cursor(),
        after.selected(),
        after.subjects.len(),
        after.place,
    );
    if after.reading_open {
        let _ = write!(line, ", reading {:?}", after.reading_title);
    }
    if let Some(picker) = &after.picker {
        let _ = write!(line, ", picker {picker:?}");
    }
    if after.compose {
        line.push_str(", composer open");
    }
    if after.digest {
        line.push_str(", digest open");
    }
    if after.filtered {
        line.push_str(", Filtered open");
    }
    if let Some(sheet) = &after.capture {
        let _ = write!(line, ", capture sheet {sheet:?}");
    }
    if let Some(toast) = &after.toast {
        let _ = write!(line, ", toast {toast:?}");
    }
    if !seen.unanswered.is_empty() {
        let ids: Vec<_> = seen.unanswered.iter().map(|id| id.as_str()).collect();
        let _ = write!(line, ", no handler for {ids:?}");
    }
    line
}

fn ok_if(condition: bool, seen: &Seen) -> Result<(), String> {
    if condition { Ok(()) } else { Err(said(seen)) }
}

fn answered(seen: &Seen) -> Result<(), String> {
    ok_if(seen.unanswered.is_empty(), seen)
}

/// Step a thread open in the dialog to an earlier message, as `p` does.
fn thread_back(window: &Window) {
    if let Some(reading) = window.reading() {
        reading.step_thread(-1);
    }
    crate::settle();
}

/// Step a digest's or Filtered's cursor down a row.
fn step_down(window: &Window) {
    if let Some(digest) = window.digest() {
        digest.step(1);
    } else if let Some(view) = window.filtered() {
        view.step(1);
    }
    crate::settle();
}

/// Open the capture sheet as a task, so `N` has a mode to switch it from.
fn as_a_task(window: &Window) {
    window.act(CommandId::CaptureTask);
    crate::settle();
}

/// The probe for `command` on `surface`, or why there is none.
fn probe(command: CommandId, surface: Surface) -> Result<Probe, &'static str> {
    use CommandId as C;
    use Surface as S;
    Ok(match (command, surface) {
        // -- Walking -----------------------------------------------------
        (C::NextMessage, S::Reader | S::Pane) => Probe::new("the next message shows", |s| {
            ok_if(s.after.reading_open && s.after.reading_title == "M02", s)
        }),
        (C::NextMessage, S::List) => Probe::new("the cursor moves down a row", |s| {
            ok_if(s.after.cursor() == s.before.cursor().map(|at| at + 1), s)
        }),
        (C::NextMessage, S::Digest) => Probe::new("the digest's cursor moves down", |s| {
            ok_if(
                s.after.digest_focus.is_some() && s.after.digest_focus != s.before.digest_focus,
                s,
            )
        }),
        (C::NextMessage, S::Filtered) => Probe::new("Filtered's cursor moves down", |s| {
            ok_if(
                s.after.filtered_focus.is_some()
                    && s.after.filtered_focus != s.before.filtered_focus,
                s,
            )
        }),
        (C::PrevMessage, S::Reader | S::Pane) => Probe::new("the message before shows", |s| {
            ok_if(s.after.reading_open && s.after.reading_title == "M01", s)
        })
        .down(1),
        (C::PrevMessage, S::List) => Probe::new("the cursor moves up a row", |s| {
            ok_if(
                s.before.cursor().is_some()
                    && s.after.cursor() == s.before.cursor().and_then(|at| at.checked_sub(1)),
                s,
            )
        })
        .down(1),
        (C::PrevMessage, S::Digest) => Probe::new("the digest's cursor moves up", |s| {
            ok_if(
                s.after.digest_focus.is_some() && s.after.digest_focus != s.before.digest_focus,
                s,
            )
        })
        .once_up(step_down),
        (C::PrevMessage, S::Filtered) => Probe::new("Filtered's cursor moves up", |s| {
            ok_if(
                s.after.filtered_focus.is_some()
                    && s.after.filtered_focus != s.before.filtered_focus,
                s,
            )
        })
        .once_up(step_down),
        (C::FirstMessage, S::Reader | S::Pane) => Probe::new("the first message shows", |s| {
            ok_if(s.after.reading_open && s.after.reading_title == "M01", s)
        })
        .down(2),
        (C::FirstMessage, S::List) => Probe::new("the cursor goes to the first row", |s| {
            ok_if(s.after.cursor() == Some(0), s)
        })
        .down(2),
        (C::LastMessage, S::Reader | S::Pane) => Probe::new("the last message shows", |s| {
            ok_if(
                s.after.reading_open && s.after.reading_title != s.before.reading_title,
                s,
            )
        }),
        (C::LastMessage, S::List) => Probe::new("the cursor goes to the last row", |s| {
            ok_if(
                s.after
                    .cursor()
                    .is_some_and(|at| at > s.before.cursor().unwrap_or(0) + 1),
                s,
            )
        }),
        (C::OpenMessage, S::Reader | S::Pane) => {
            return Err("already open: Return scrolls it (open_keys)");
        }
        (C::OpenMessage, S::List | S::Menu | S::Digest | S::Filtered) => {
            Probe::new("the message opens", |s| ok_if(s.after.reading_open, s))
        }
        (C::ToggleSelection, S::List | S::Reader | S::Pane) => {
            Probe::new("one more row is selected", |s| {
                ok_if(s.after.selected() == s.before.selected() + 1, s)
            })
        }
        (C::ExtendSelectionDown, S::List | S::Reader | S::Pane) => {
            Probe::new("the selection grows downward", |s| {
                ok_if(
                    s.after.selected() >= 2 || s.after.cursor() > s.before.cursor(),
                    s,
                )
            })
        }
        (C::ExtendSelectionUp, S::List | S::Reader | S::Pane) => {
            Probe::new("the selection grows upward", |s| {
                ok_if(
                    s.after.selected() >= 2 || s.after.cursor() < s.before.cursor(),
                    s,
                )
            })
            .down(2)
        }
        (C::SelectAll, S::List | S::Reader | S::Pane) => Probe::new("every row is selected", |s| {
            ok_if(s.after.selected() as usize >= s.rows, s)
        }),
        (C::PrevView, S::Reader | S::Pane) => {
            Probe::new("the message closes, the list is back", |s| {
                ok_if(!s.after.reading_open, s)
            })
        }
        (C::PrevView, S::List) => Probe::new("the key is answered", answered),
        (C::NextInConversation, S::Reader | S::Pane) => {
            Probe::new("the thread steps to a later message", |s| {
                ok_if(
                    s.after.reading_open && s.after.reading_body != s.before.reading_body,
                    s,
                )
            })
            .on(Flavor::Thread)
            .once_up(thread_back)
        }
        (C::PrevInConversation, S::Reader | S::Pane) => {
            Probe::new("the thread steps to an earlier message", |s| {
                ok_if(
                    s.after.reading_open && s.after.reading_body != s.before.reading_body,
                    s,
                )
            })
            .on(Flavor::Thread)
        }
        (C::ToggleFold | C::ExpandAll, _) => {
            return Err("Conversation is a stacked-pane context; GTK has no such surface");
        }
        // -- Verbs ---------------------------------------------------------
        (C::Archive, S::List | S::Reader | S::Pane | S::Menu)
        | (C::ArchiveThread, S::List | S::Reader | S::Pane) => {
            Probe::new("the message leaves the inbox for the archive", |s| {
                ok_if(s.after.mailbox() == s.archive && s.archive.is_some(), s)
            })
        }
        (C::ArchiveThread, S::Digest) => {
            Probe::new("everything the digest holds is archived", |s| {
                ok_if(s.after.mailbox() == s.archive && s.archive.is_some(), s)
            })
            .fresh()
        }
        (C::Delete, S::List | S::Menu | S::Reader | S::Pane | S::Filtered) => {
            let probe = Probe::new("the message goes to the trash", |s| {
                ok_if(s.after.mailbox() == s.trash && s.trash.is_some(), s)
            });
            if surface == S::Filtered {
                probe.fresh()
            } else {
                probe
            }
        }
        (C::Move, S::List | S::Menu | S::Reader | S::Pane) => {
            Probe::new("the move picker opens", |s| {
                ok_if(
                    s.after
                        .picker
                        .as_ref()
                        .is_some_and(|text| text.contains("move")),
                    s,
                )
            })
        }
        (C::AddLabel, S::List | S::Menu | S::Reader | S::Pane) => {
            Probe::new("the label picker opens", |s| {
                ok_if(
                    s.after
                        .picker
                        .as_ref()
                        .is_some_and(|text| text.contains("label")),
                    s,
                )
            })
        }
        (C::Snooze, S::List | S::Menu | S::Reader | S::Pane) => {
            Probe::new("the snooze picker opens", |s| {
                ok_if(
                    s.after
                        .picker
                        .as_ref()
                        .is_some_and(|text| text.contains("snooze") || text.contains("until")),
                    s,
                )
            })
        }
        (C::Flag, S::List | S::Menu | S::Reader | S::Pane | S::Filtered) => {
            Probe::new("the message's flag flips", |s| {
                ok_if(s.after.has(Flag::Flagged) != s.before.has(Flag::Flagged), s)
            })
        }
        (C::ToggleRead, S::List | S::Menu | S::Reader | S::Pane | S::Filtered) => {
            Probe::new("the message's read state flips", |s| {
                ok_if(s.after.has(Flag::Seen) != s.before.has(Flag::Seen), s)
            })
        }
        (C::Undo, S::List | S::Reader | S::Pane | S::Digest | S::Filtered) => {
            let probe = Probe::new("what was just done is taken back", |s| {
                ok_if(
                    s.before.mailbox().is_some() && s.after.mailbox() != s.before.mailbox(),
                    s,
                )
            })
            .after(if surface == S::Filtered {
                C::Delete
            } else {
                C::Archive
            });
            if matches!(surface, S::Digest | S::Filtered) {
                probe.fresh()
            } else {
                probe
            }
        }
        // -- What opens something ------------------------------------------
        (C::Reply | C::ReplyAll | C::Forward, S::List | S::Menu | S::Reader | S::Pane)
        | (C::Compose, S::List | S::Reader | S::Pane) => {
            Probe::new("the composer opens", |s| ok_if(s.after.compose, s))
        }
        (C::ViewSource, S::List | S::Reader | S::Pane) => {
            Probe::new("the raw source opens", |s| ok_if(s.after.source, s))
        }
        (C::OpenAttachmentOrLink, S::List | S::Reader | S::Pane) => {
            Probe::new("the attachments are offered", |s| {
                ok_if(s.after.choices > 0, s)
            })
            .on(Flavor::Attachment)
        }
        (C::Unsubscribe, S::List | S::Reader | S::Pane) => {
            Probe::new("the unsubscribe is logged", |s| {
                ok_if(s.after.unsubscribed == s.before.unsubscribed + 1, s)
            })
            .on(Flavor::Newsletter)
        }
        (C::Unsubscribe, S::Digest) => Probe::new("the unsubscribe is logged", |s| {
            ok_if(s.after.unsubscribed == s.before.unsubscribed + 1, s)
        }),
        (C::CaptureTask | C::CaptureNote, S::List | S::Reader | S::Pane) => {
            Probe::new("the capture sheet opens", |s| {
                ok_if(s.after.capture.is_some(), s)
            })
            .on(Flavor::Vault)
        }
        (C::CaptureNote, S::Capture) => Probe::new("the sheet is a note's", |s| {
            ok_if(
                s.after
                    .capture
                    .as_ref()
                    .is_some_and(|texts| texts.iter().any(|text| text == "Add note")),
                s,
            )
        })
        .once_up(as_a_task),
        (C::CaptureTask, S::Capture) => Probe::new("the sheet is a task's", |s| {
            ok_if(
                s.after
                    .capture
                    .as_ref()
                    .is_some_and(|texts| texts.iter().any(|text| text == "Add task")),
                s,
            )
        }),
        (C::MoreActions, S::Reader | S::Pane) => {
            Probe::new("the More menu opens", |s| ok_if(s.after.more, s))
        }
        (C::Search, S::List | S::Reader | S::Pane) => {
            Probe::new("the search bar opens", |s| ok_if(s.after.bar, s))
        }
        (C::CheatSheet, S::List | S::Reader | S::Pane) => {
            Probe::new("the key map opens", |s| ok_if(s.after.key_map, s))
        }
        // -- Going places --------------------------------------------------
        (C::GoToFolders, S::List | S::Reader | S::Pane) => {
            Probe::new("the folders open", |s| ok_if(s.after.places, s))
        }
        (C::GoToInbox, S::Digest) => Probe::new("the digest closes", |s| ok_if(!s.after.digest, s)),
        (C::GoToInbox, S::Filtered) => {
            Probe::new("Filtered closes", |s| ok_if(!s.after.filtered, s))
        }
        (C::GoToInbox, S::List) => Probe::new("the inbox is showing", |s| {
            ok_if(s.after.place == "Inbox", s)
        })
        .after(C::GoToSent),
        (C::GoToInbox, S::Reader | S::Pane) => {
            Probe::new("the inbox is showing, nothing unanswered", |s| {
                ok_if(s.after.place == "Inbox" && s.unanswered.is_empty(), s)
            })
        }
        (C::GoToDrafts, S::List | S::Reader | S::Pane) => place("Drafts"),
        (C::GoToSent, S::List | S::Reader | S::Pane) => place("Sent"),
        (C::GoToFlagged, S::List | S::Reader | S::Pane) => place("Flagged"),
        (C::GoToArchive, S::List | S::Reader | S::Pane) => place("Archive"),
        (C::GoToSnoozed, S::List | S::Reader | S::Pane) => place("Snoozed"),
        (C::GoToOutbox, S::List | S::Reader | S::Pane) => place(postio_ui::places::OUTBOX),
        (C::GoToJunk, S::List | S::Reader | S::Pane) => place("Junk"),
        (C::GoToTrash, S::List | S::Reader | S::Pane) => place("Trash"),
        (C::GoToFiltered, S::List | S::Reader | S::Pane) => {
            Probe::new("Filtered opens", |s| ok_if(s.after.filtered, s))
        }
        (C::GoToDigestRules, S::List | S::Reader | S::Pane) => {
            Probe::new("the digest rules open", |s| ok_if(s.after.rules, s))
        }
        (C::NextScope, S::List | S::Reader | S::Pane) => {
            Probe::new("the key is answered", answered)
        }
        _ => return Err("not a Gmail key in this surface"),
    })
}

/// A probe for a `g` destination: the place's name is what the header says.
fn place(name: &'static str) -> Probe {
    // A closure cannot capture, so the destinations are matched by a table.
    match name {
        "Drafts" => Probe::new("Drafts is showing", |s| ok_if(s.after.place == "Drafts", s)),
        "Sent" => Probe::new("Sent is showing", |s| ok_if(s.after.place == "Sent", s)),
        "Flagged" => Probe::new("Flagged is showing", |s| {
            ok_if(s.after.place == "Flagged", s)
        }),
        "Archive" => Probe::new("Archive is showing", |s| {
            ok_if(s.after.place == "Archive", s)
        }),
        "Snoozed" => Probe::new("Snoozed is showing", |s| {
            ok_if(s.after.place == "Snoozed", s)
        }),
        "Junk" => Probe::new("Junk is showing", |s| ok_if(s.after.place == "Junk", s)),
        "Trash" => Probe::new("Trash is showing", |s| ok_if(s.after.place == "Trash", s)),
        _ => Probe::new("the outbox is showing", |s| {
            ok_if(s.after.place == postio_ui::places::OUTBOX, s)
        }),
    }
}

/// The commands that carry a Gmail key (docs/PRODUCT.md §8), in the
/// registry's order.
const GMAIL: &[CommandId] = &[
    CommandId::NextMessage,
    CommandId::PrevMessage,
    CommandId::FirstMessage,
    CommandId::LastMessage,
    CommandId::OpenMessage,
    CommandId::ToggleSelection,
    CommandId::ExtendSelectionDown,
    CommandId::ExtendSelectionUp,
    CommandId::SelectAll,
    CommandId::PrevView,
    CommandId::NextInConversation,
    CommandId::PrevInConversation,
    CommandId::ToggleFold,
    CommandId::ExpandAll,
    CommandId::Reply,
    CommandId::ReplyAll,
    CommandId::Forward,
    CommandId::Compose,
    CommandId::Archive,
    CommandId::ArchiveThread,
    CommandId::Delete,
    CommandId::Move,
    CommandId::AddLabel,
    CommandId::Flag,
    CommandId::ToggleRead,
    CommandId::Snooze,
    CommandId::Undo,
    CommandId::ViewSource,
    CommandId::OpenAttachmentOrLink,
    CommandId::Unsubscribe,
    CommandId::CaptureTask,
    CommandId::CaptureNote,
    CommandId::MoreActions,
    CommandId::Search,
    CommandId::CheatSheet,
    CommandId::GoToFolders,
    CommandId::GoToInbox,
    CommandId::GoToDrafts,
    CommandId::GoToSent,
    CommandId::GoToFlagged,
    CommandId::GoToArchive,
    CommandId::GoToSnoozed,
    CommandId::GoToOutbox,
    CommandId::GoToJunk,
    CommandId::GoToTrash,
    CommandId::GoToFiltered,
    CommandId::GoToDigestRules,
    CommandId::NextScope,
];

/// Presses that are in the registry and are not run here, each with the
/// reason. A key that is here and is no longer bound fails the completeness
/// case, so the list cannot go stale.
const EXCUSED: &[(Surface, CommandId, &str, &str)] = &[
    (
        Surface::Reader,
        CommandId::NextMessage,
        "Down",
        "the open message scrolls with the arrows (T196); open_keys presses them",
    ),
    (
        Surface::Pane,
        CommandId::NextMessage,
        "Down",
        "the open message scrolls with the arrows (T196); open_keys presses them",
    ),
    (
        Surface::Reader,
        CommandId::PrevMessage,
        "Up",
        "the open message scrolls with the arrows (T196); open_keys presses them",
    ),
    (
        Surface::Pane,
        CommandId::PrevMessage,
        "Up",
        "the open message scrolls with the arrows (T196); open_keys presses them",
    ),
    (
        Surface::Reader,
        CommandId::MoreActions,
        ".",
        "More exists only where the dialog is folded; open_layout presses it there",
    ),
    (
        Surface::Pane,
        CommandId::MoreActions,
        ".",
        "More exists only where the dialog is folded; open_layout presses it there",
    ),
    (
        Surface::List,
        CommandId::PrevView,
        "Left",
        "nothing is open to go back from in the list",
    ),
    (
        Surface::List,
        CommandId::PrevView,
        "u",
        "nothing is open to go back from in the list",
    ),
    (
        Surface::List,
        CommandId::OpenAttachmentOrLink,
        "A",
        "it offers the attachments and links of the message open, and a list row is \
         not one: the controller has no rule for it on a row, and the terminal \
         offers choices only for an open message (the open message presses it)",
    ),
    (
        Surface::List,
        CommandId::Unsubscribe,
        "M",
        "it leaves the list of the message open, and a list row is not one: the Mac \
         asks for the message window, and the terminal for an open message (the open \
         message presses it)",
    ),
    (
        Surface::Capture,
        CommandId::CaptureTask,
        "T",
        "the sheet's keyboard is in its text field, where a capital types; the \
         terminal makes the same choice, and the Task toggle is the button the \
         controller's `capture_task` answers",
    ),
    (
        Surface::Capture,
        CommandId::CaptureNote,
        "N",
        "the sheet's keyboard is in its text field, where a capital types; the \
         terminal makes the same choice, and the Note toggle is the button the \
         controller's `capture_note` answers",
    ),
];

fn excused(surface: Surface, command: CommandId, binding: &str) -> bool {
    EXCUSED
        .iter()
        .any(|(s, c, b, _)| *s == surface && *c == command && *b == binding)
}

/// A failed press: the key, the surface, what should have happened, what did.
struct Red {
    surface: Surface,
    command: CommandId,
    key: String,
    expected: &'static str,
    actual: String,
}

impl std::fmt::Display for Red {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}] {} `{}`: expected {}; saw {}",
            self.surface,
            self.command.as_str(),
            self.key,
            self.expected,
            self.actual
        )
    }
}

/// The commands pressed together, so that no case outlasts the runner's
/// patience and a red one names its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    /// Walking, opening, selecting.
    Walk,
    /// The verbs on a message: archive, delete, flag, read, snooze, move,
    /// label, undo.
    Verbs,
    /// The composer: reply, reply all, forward, compose.
    Writes,
    /// What opens something: the source, the attachments, the unsubscribe,
    /// the capture sheet, search, the key map.
    Opens,
    /// The `g` destinations.
    Go,
}

impl Group {
    fn of(command: CommandId) -> Group {
        use CommandId as C;
        match command {
            C::NextMessage
            | C::PrevMessage
            | C::FirstMessage
            | C::LastMessage
            | C::OpenMessage
            | C::PrevView
            | C::NextInConversation
            | C::PrevInConversation
            | C::ToggleFold
            | C::ExpandAll
            | C::ToggleSelection
            | C::ExtendSelectionDown
            | C::ExtendSelectionUp
            | C::SelectAll => Group::Walk,
            C::Archive
            | C::ArchiveThread
            | C::Delete
            | C::Move
            | C::AddLabel
            | C::Flag
            | C::ToggleRead
            | C::Snooze
            | C::Undo => Group::Verbs,
            C::Reply | C::ReplyAll | C::Forward | C::Compose => Group::Writes,
            C::GoToFolders
            | C::GoToInbox
            | C::GoToDrafts
            | C::GoToSent
            | C::GoToFlagged
            | C::GoToArchive
            | C::GoToSnoozed
            | C::GoToOutbox
            | C::GoToJunk
            | C::GoToTrash
            | C::GoToFiltered
            | C::GoToDigestRules
            | C::NextScope => Group::Go,
            _ => Group::Opens,
        }
    }
}

/// Every (command, binding, probe) that `surface` offers in `group`.
fn presses(surface: Surface, group: Group) -> Vec<(CommandId, String, Probe)> {
    let mut all = Vec::new();
    for command in GMAIL.iter().filter(|command| Group::of(**command) == group) {
        let spec = registry::all()
            .find(|spec| spec.id == *command)
            .expect("a registered command");
        if !spec.contexts.contains(surface.context()) || probe(*command, surface).is_err() {
            continue;
        }
        for binding in bindings_of(*command) {
            if excused(surface, *command, &binding) {
                continue;
            }
            let mut probe = probe(*command, surface).expect("the same probe");
            // Digest and Filtered have a world of their own whatever the
            // probe says.
            probe.flavor = match surface {
                Surface::Digest => Flavor::Digest,
                Surface::Filtered => Flavor::Filtered,
                _ => probe.flavor,
            };
            all.push((*command, binding, probe));
        }
    }
    all
}

/// Press every binding of every Gmail command in `group` that `surface`
/// offers, and answer what did not do what it should.
async fn press_all(surface: Surface, group: Group) -> Vec<Red> {
    let mut reds = Vec::new();
    let mut pressed = 0;
    let all = presses(surface, group);
    for flavor in [
        Flavor::Plain,
        Flavor::Attachment,
        Flavor::Newsletter,
        Flavor::Thread,
        Flavor::Vault,
        Flavor::Digest,
        Flavor::Filtered,
    ] {
        let mut slot: Option<World> = None;
        for (command, binding, probe_for) in all.iter().filter(|(_, _, p)| p.flavor == flavor) {
            if probe_for.fresh
                && let Some(spent) = slot.take()
            {
                spent.window.destroy();
            }
            if slot.is_none() {
                slot = Some(World::new(flavor, surface).await);
            }
            let world = slot.as_ref().expect("a world");
            if let Some(red) = press_one(world, surface, *command, binding, probe_for).await {
                reds.push(red);
            }
            pressed += 1;
            if probe_for.fresh
                && let Some(spent) = slot.take()
            {
                spent.window.destroy();
            }
        }
    }
    eprintln!(
        "{surface:?} {group:?}: {pressed} keys pressed, {} red",
        reds.len()
    );
    reds
}

async fn press_one(
    world: &World,
    surface: Surface,
    command: CommandId,
    binding: &str,
    probe: &Probe,
) -> Option<Red> {
    let window = &world.window;
    let started = std::time::Instant::now();
    world.reset().await;
    let walked = probe.cursor_down as u32;
    for _ in 0..probe.cursor_down {
        window.act(CommandId::NextMessage);
        crate::settle();
    }
    assert!(
        crate::settle_until(async || window.observe().cursor.index == Some(walked)).await,
        "{surface:?}: the cursor never reached row {walked}: {:?}",
        window.observe().cursor
    );
    // What the verbs act on before the surface comes up: the list's row.
    // Filtered acts on what it holds, so it is acted on once it is up.
    let up_first = surface != Surface::Filtered;
    let mut watch = world.aimed();
    let left = async |watch: Option<MessageId>, was: Option<MailboxId>| {
        if let (Some(message), Some(was)) = (watch, was) {
            let moved =
                crate::settle_until(async || world.stored(message).await.mailbox != was).await;
            assert!(
                moved,
                "{surface:?}: {message:?} never left {was:?} for the setup"
            );
        }
        crate::settle();
    };
    if up_first && let Some(prior) = probe.before {
        let was = match watch {
            Some(message) => Some(world.stored(message).await.mailbox),
            None => None,
        };
        window.act(prior);
        if prior == CommandId::GoToSent {
            crate::settle_for(std::time::Duration::from_millis(300)).await;
        } else {
            left(watch, was).await;
        }
    }
    world.enter(surface).await;
    if surface == Surface::Filtered {
        watch = window.filtered().and_then(|view| view.focused());
    }
    if !up_first && let Some(prior) = probe.before {
        let was = match watch {
            Some(message) => Some(world.stored(message).await.mailbox),
            None => None,
        };
        window.act(prior);
        left(watch, was).await;
    }
    if surface == Surface::Digest && probe.before.is_none() {
        watch = world.held.first().copied();
    }
    if probe.flavor == Flavor::Thread
        && let Some(reading) = window.reading()
    {
        // The conversation's other messages arrive a moment after the first.
        let known = crate::settle_until(async || reading.thread_known() == 3).await;
        assert!(known, "the thread's messages were never read");
        assert!(world.drawn().await, "the open message was never drawn");
    }
    if let Some(step) = probe.once_up {
        step(window);
        if probe.flavor == Flavor::Thread {
            assert!(
                world.drawn().await,
                "the message the step went to was never drawn"
            );
        }
    }
    window.take_unanswered();
    let before = Snap::take(world, watch).await;
    if surface == Surface::Capture {
        eprintln!("  keyboard on {}", support::focus_path(window));
    }
    let taken = press_binding(window, binding);
    let rows = before.subjects.len();
    let last = RefCell::new(String::new());
    let unanswered = RefCell::new(Vec::new());
    let green = crate::settle_until_within(std::time::Duration::from_secs(5), async || {
        let after = Snap::take(world, watch).await;
        unanswered.borrow_mut().extend(window.take_unanswered());
        let unanswered = unanswered.borrow();
        let seen = Seen {
            before: &before,
            after: &after,
            archive: world.archive,
            trash: world.trash,
            unanswered: &unanswered,
            rows,
        };
        match (probe.verdict)(&seen) {
            Ok(()) => true,
            Err(why) => {
                *last.borrow_mut() = why;
                false
            }
        }
    })
    .await;
    world.reset().await;
    eprintln!(
        "{surface:?} {} `{binding}`: {} in {:?}",
        command.as_str(),
        if green { "green" } else { "RED" },
        started.elapsed()
    );
    (!green).then(|| Red {
        surface,
        command,
        key: binding.to_owned(),
        expected: probe.expected,
        actual: format!(
            "{} (the last key was {})",
            last.into_inner(),
            if taken { "taken" } else { "not taken" }
        ),
    })
}

fn report(reds: &[Red]) -> String {
    reds.iter().map(|red| format!("\n  {red}")).collect()
}

/// One case per surface and group, each reporting every red press at once.
macro_rules! cases {
    ($($name:ident: $surface:ident, $group:ident;)*) => {
        $(
            pub fn $name() {
                crate::gtk_case(async {
                    if !support::display() {
                        return;
                    }
                    let reds = press_all(Surface::$surface, Group::$group).await;
                    assert!(reds.is_empty(), "red:{}", report(&reds));
                });
            }
        )*
    };
}

cases! {
    list_walks: List, Walk;
    list_verbs: List, Verbs;
    list_writes: List, Writes;
    list_opens: List, Opens;
    list_goes: List, Go;
    reader_walks: Reader, Walk;
    reader_verbs: Reader, Verbs;
    reader_writes: Reader, Writes;
    reader_opens: Reader, Opens;
    reader_goes: Reader, Go;
    pane_walks: Pane, Walk;
    pane_verbs: Pane, Verbs;
    pane_writes: Pane, Writes;
    pane_opens: Pane, Opens;
    pane_goes: Pane, Go;
    digest_walks: Digest, Walk;
    digest_verbs: Digest, Verbs;
    digest_opens: Digest, Opens;
    digest_goes: Digest, Go;
    filtered_walks: Filtered, Walk;
    filtered_verbs: Filtered, Verbs;
    filtered_goes: Filtered, Go;
    capture_opens: Capture, Opens;
    menu_walks: Menu, Walk;
    menu_verbs: Menu, Verbs;
    menu_writes: Menu, Writes;
}

/// Every Gmail command is pressed somewhere, or says why not. The registry
/// is the source of the contexts, so a command that gains one is caught
/// here, not left unpressed.
pub fn every_gmail_command_is_probed_or_excused() {
    let mut unprobed = Vec::new();
    for command in GMAIL {
        let spec = registry::all()
            .find(|spec| spec.id == *command)
            .expect("a registered command");
        let probed = Surface::ALL.into_iter().any(|surface| {
            spec.contexts.contains(surface.context()) && probe(*command, surface).is_ok()
        });
        if !probed {
            unprobed.push(command.as_str());
        }
    }
    // Conversation is a stacked-pane context GTK does not have.
    let allowed = ["toggle_fold", "expand_all"];
    unprobed.retain(|id| !allowed.contains(id));
    assert!(
        unprobed.is_empty(),
        "Gmail commands no surface presses: {unprobed:?}"
    );
    for (surface, command, binding, why) in EXCUSED {
        assert!(
            bindings_of(*command).iter().any(|b| b == binding),
            "{why}: {command:?} no longer has {binding}; take the excuse out ({surface:?})"
        );
    }
    // Every Gmail key the contract names is bound to the command it names.
    for (key, command) in [
        ("e", CommandId::Archive),
        ("r", CommandId::Reply),
        ("a", CommandId::ReplyAll),
        ("f", CommandId::Forward),
        ("#", CommandId::Delete),
        ("s", CommandId::Flag),
        ("b", CommandId::Snooze),
        ("v", CommandId::Move),
        ("I", CommandId::ToggleRead),
        ("U", CommandId::ToggleRead),
        ("z", CommandId::Undo),
        ("o", CommandId::OpenMessage),
        ("u", CommandId::PrevView),
        ("j", CommandId::NextMessage),
        ("k", CommandId::PrevMessage),
        ("n", CommandId::NextInConversation),
        ("p", CommandId::PrevInConversation),
        (";", CommandId::ExpandAll),
        ("x", CommandId::ToggleSelection),
        ("* a", CommandId::SelectAll),
        ("T", CommandId::CaptureTask),
        ("E", CommandId::ArchiveThread),
        ("V", CommandId::ViewSource),
        ("A", CommandId::OpenAttachmentOrLink),
        ("N", CommandId::CaptureNote),
        ("M", CommandId::Unsubscribe),
        ("Z", CommandId::ToggleFold),
        ("g o", CommandId::GoToOutbox),
        ("g r", CommandId::GoToDigestRules),
        ("g c", CommandId::NextScope),
        ("g i", CommandId::GoToInbox),
        ("g s", CommandId::GoToFlagged),
        ("g t", CommandId::GoToSent),
        ("g d", CommandId::GoToDrafts),
        ("g a", CommandId::GoToArchive),
        ("g b", CommandId::GoToSnoozed),
        ("g l", CommandId::GoToFolders),
        ("mod+z", CommandId::Undo),
        ("Delete", CommandId::Delete),
        ("X", CommandId::SelectAll),
    ] {
        let expanded =
            postio_config::keys::expand_mod(key, postio_config::paths::Platform::Freedesktop);
        assert!(
            bindings_of(command).contains(&expanded),
            "{command:?} does not answer to {key}: {:?}",
            bindings_of(command)
        );
    }
}

// ---------------------------------------------------------------------------
// What the window says about the keys
// ---------------------------------------------------------------------------

/// The keys Gmail's adoption retired, per command: what each was before
/// 2026-10-14 and is not now. Nothing the window teaches may still say them
/// for that command.
const RETIRED: &[(CommandId, &str)] = &[
    (CommandId::Archive, "a"),
    (CommandId::ArchiveThread, "A"),
    (CommandId::Reply, "e"),
    (CommandId::ReplyAll, "E"),
    (CommandId::Move, "m"),
    (CommandId::Flag, "*"),
    (CommandId::ToggleRead, "r"),
    (CommandId::Snooze, "s"),
    (CommandId::ViewSource, "v"),
    (CommandId::OpenAttachmentOrLink, "o"),
    (CommandId::Unsubscribe, "U"),
    (CommandId::NextInConversation, "]"),
    (CommandId::PrevInConversation, "["),
    (CommandId::ToggleFold, "z"),
    (CommandId::ExpandAll, "O"),
    (CommandId::CaptureTask, "t"),
    (CommandId::CaptureNote, "n"),
    (CommandId::GoToFolders, "g o"),
    (CommandId::GoToDrafts, "g t"),
    (CommandId::GoToSent, "g s"),
    (CommandId::GoToFlagged, "g *"),
    (CommandId::GoToArchive, "g r"),
    (CommandId::GoToSnoozed, "g z"),
    (CommandId::GoToOutbox, "g b"),
    (CommandId::GoToDigestRules, "g d"),
    (CommandId::NextScope, "g a"),
];

/// The key `command` is taught by: its primary binding, as the registry
/// spells it for this machine.
fn taught(command: CommandId) -> String {
    Keymap::defaults()
        .binding(command)
        .unwrap_or_else(|| panic!("{command:?} has no binding"))
        .to_owned()
}

/// Each keycap under `root` that is on screen, with the words of the control
/// it sits in: the first label, and all of them.
fn caps_beside(root: &impl IsA<gtk::Widget>) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    for widget in support::descendants(root) {
        if !(widget.has_css_class("postio-keyhint") && widget.is_mapped()) {
            continue;
        }
        let Some(cap) = widget.downcast_ref::<gtk::Label>() else {
            continue;
        };
        let mut up = widget.parent();
        while let Some(parent) = up {
            let texts: Vec<String> = support::descendants(&parent)
                .iter()
                .filter(|w| !w.has_css_class("postio-keyhint"))
                .filter_map(|w| w.downcast_ref::<gtk::Label>().map(|l| l.text().to_string()))
                .filter(|t| !t.is_empty())
                .collect();
            if !texts.is_empty() {
                found.push((texts[0].clone(), cap.text().to_string(), texts.join(" / ")));
                break;
            }
            up = parent.parent();
        }
    }
    found
}

/// Words a control says, and the commands whose key may sit beside them.
type Words<'a> = &'a [(&'a str, &'a [CommandId])];

/// Check that every control in `words` is on screen under `root`, and that
/// the key beside it is the registry's for the command it names.
fn teaches(scene: &str, root: &impl IsA<gtk::Widget>, words: Words, problems: &mut Vec<String>) {
    let pairs = caps_beside(root);
    for (said, commands) in words {
        let found: Vec<_> = pairs
            .iter()
            .filter(|(first, _, _)| first == said || first.starts_with(&format!("{said} \u{b7}")))
            .collect();
        if found.is_empty() {
            problems.push(format!("{scene}: no control says {said:?} with a key"));
        }
        for (first, cap, _) in found {
            let allowed: Vec<String> = commands.iter().map(|command| taught(*command)).collect();
            if !allowed.contains(cap) {
                problems.push(format!(
                    "{scene}: {first:?} shows {cap:?}; the keymap's key for {commands:?} is {allowed:?}"
                ));
            }
        }
    }
}

use CommandId as K;

/// The verbs the message dialog, the row menu and the bulk bar draw.
const VERBS: Words = &[
    ("Reply", &[K::Reply]),
    ("Reply all", &[K::ReplyAll]),
    ("Forward", &[K::Forward]),
    ("Archive", &[K::Archive]),
    ("Snooze", &[K::Snooze]),
    ("Snooze\u{2026}", &[K::Snooze]),
    ("Remind", &[K::RemindIfNoReply]),
    ("Remind if no reply\u{2026}", &[K::RemindIfNoReply]),
    ("Label", &[K::AddLabel]),
    ("Label\u{2026}", &[K::AddLabel]),
    ("Move", &[K::Move]),
    ("Move\u{2026}", &[K::Move]),
    ("Delete", &[K::Delete]),
];

/// The strings that only some of those scenes draw; a scene says which.
fn only<'a>(all: Words<'a>, wanted: &[&str]) -> Vec<(&'a str, &'a [CommandId])> {
    all.iter()
        .filter(|(said, _)| wanted.contains(said))
        .copied()
        .collect()
}

pub fn the_controls_teach_the_gmail_keys_and_no_old_ones() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let mut problems = Vec::new();

        // The chrome, the bulk bar a selection brings, the toast an archive
        // leaves, and the folders.
        let world = World::new(Flavor::Plain, Surface::List).await;
        let window = &world.window;
        teaches(
            "the header",
            window,
            &[
                ("Inbox", &[K::GoToFolders]),
                ("Has action", &[K::ToggleHasAction]),
            ],
            &mut problems,
        );
        let compose = postio_ui::focus_row::compose_tooltip(Some(&taught(K::Compose)));
        let tips: Vec<String> = support::descendants(window)
            .iter()
            .filter_map(|widget| widget.tooltip_text().map(|tip| tip.to_string()))
            .collect();
        if !tips.contains(&compose) {
            problems.push(format!(
                "the header: Compose's tooltip is not {compose:?}: {tips:?}"
            ));
        }

        window.act(CommandId::ToggleSelection);
        crate::settle();
        let mut bulk = only(VERBS, &["Archive", "Snooze", "Label", "Move", "Delete"]);
        bulk.extend([
            ("Mark read", &[K::ToggleRead][..]),
            ("Digest these\u{2026}", &[K::DigestRule][..]),
        ]);
        teaches("the bulk bar", window, &bulk, &mut problems);
        world.reset().await;

        window.act(CommandId::Archive);
        crate::settle_until(async || window.toast_showing().is_some()).await;
        teaches("the toast", window, &[("Undo", &[K::Undo])], &mut problems);
        world.reset().await;

        window.act(CommandId::GoToFolders);
        crate::settle_until(async || {
            window
                .places()
                .is_some_and(|places| places.names().len() >= 9)
        })
        .await;
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        teaches(
            "the folders",
            window,
            &[
                ("Drafts", &[K::GoToDrafts]),
                ("Sent", &[K::GoToSent]),
                ("Snoozed", &[K::GoToSnoozed]),
                ("Archive", &[K::GoToArchive]),
                ("Filtered", &[K::GoToFiltered]),
                ("Flagged", &[K::GoToFlagged]),
                ("Junk", &[K::GoToJunk]),
                ("Trash", &[K::GoToTrash]),
            ],
            &mut problems,
        );
        world.reset().await;

        // The row's menu.
        window.open_row_menu(0, gtk::gdk::Rectangle::new(40, 120, 1, 1));
        crate::settle();
        if let Some(menu) = window.row_menu() {
            let mut menu_words = only(
                VERBS,
                &[
                    "Reply",
                    "Reply all",
                    "Forward",
                    "Archive",
                    "Snooze\u{2026}",
                    "Remind if no reply\u{2026}",
                    "Label\u{2026}",
                    "Move\u{2026}",
                    "Delete",
                ],
            );
            menu_words.extend([
                ("Open", &[K::OpenMessage][..]),
                ("Mark read", &[K::ToggleRead][..]),
                ("Flag", &[K::Flag][..]),
                ("Digest mail like this\u{2026}", &[K::DigestRule][..]),
            ]);
            teaches("the row menu", menu.widget(), &menu_words, &mut problems);
        } else {
            problems.push("the row menu never opened".to_owned());
        }
        world.reset().await;

        // The open message.
        world.enter(Surface::Reader).await;
        let mut reading = only(
            VERBS,
            &[
                "Reply",
                "Reply all",
                "Forward",
                "Archive",
                "Snooze",
                "Remind",
                "Label",
                "Move",
                "Delete",
            ],
        );
        reading.push(("+ Label", &[K::AddLabel]));
        teaches("the open message", window, &reading, &mut problems);
        let steps: Vec<String> = caps_beside(window)
            .into_iter()
            .filter(|(_, _, all)| all.contains("Message 1 of"))
            .map(|(_, cap, _)| cap)
            .collect();
        let walking = [taught(K::PrevMessage), taught(K::NextMessage)];
        if !walking.iter().all(|key| steps.contains(key)) {
            problems.push(format!(
                "the open message: its previous and next show {steps:?}, not {walking:?}"
            ));
        }
        world.reset().await;
        window.act(CommandId::CaptureTask);
        crate::settle_until(async || window.capture().is_some_and(|sheet| sheet.is_shown())).await;
        // The capture sheet is not open without a vault; the toast says why.
        world.reset().await;

        assert!(
            problems.is_empty(),
            "the window teaches:\n  {}",
            problems.join("\n  ")
        );
    });
}

pub fn the_attachment_and_unsubscribe_controls_teach_their_keys() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let mut problems = Vec::new();
        let world = World::new(Flavor::Newsletter, Surface::Reader).await;
        world.enter(Surface::Reader).await;
        crate::settle_until(async || {
            caps_beside(&world.window)
                .iter()
                .any(|(first, _, _)| first == "Unsubscribe")
        })
        .await;
        teaches(
            "the unsubscribe notice",
            &world.window,
            &[("Unsubscribe", &[K::Unsubscribe])],
            &mut problems,
        );
        world.reset().await;

        let world = World::new(Flavor::Vault, Surface::Capture).await;
        world.window.act(CommandId::CaptureTask);
        crate::settle_until(async || world.window.capture().is_some_and(|sheet| sheet.is_shown()))
            .await;
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        teaches(
            "the capture sheet",
            &world.window,
            &[("Task", &[K::CaptureTask]), ("Note", &[K::CaptureNote])],
            &mut problems,
        );

        // The page an empty inbox shows.
        let fixture = Fixture::empty().await;
        let window = Window::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        ));
        crate::settle_until(async || {
            caps_beside(&window)
                .iter()
                .any(|(first, _, _)| first == "compose")
        })
        .await;
        teaches(
            "the empty inbox",
            &window,
            &[
                ("archive", &[K::GoToArchive]),
                ("compose", &[K::Compose]),
                ("0 filtered today", &[K::GoToFiltered]),
            ],
            &mut problems,
        );
        assert!(
            problems.is_empty(),
            "the window teaches:\n  {}",
            problems.join("\n  ")
        );
    });
}

/// `?`'s key map: each Gmail command's row shows every key the registry
/// gives it, and none that it had before.
pub fn the_key_map_shows_the_gmail_keys_and_no_old_ones() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let world = World::new(Flavor::Plain, Surface::List).await;
        let window = &world.window;
        press_binding(window, "?");
        assert!(
            crate::settle_until(async || window.key_map().is_some()).await,
            "? did not open the key map"
        );
        let dialog = window.key_map().expect("the key map");
        let rows_shown = || -> Vec<(String, Vec<String>)> {
            support::with_class(&dialog, "focus-keymap-row")
                .iter()
                .map(|row| {
                    let title = support::texts(&support::only(row, "focus-keymap-title")).join(" ");
                    let keys = support::with_class(row, "postio-keyhint")
                        .iter()
                        .flat_map(support::texts)
                        .collect();
                    (title, keys)
                })
                .collect()
        };
        assert!(
            crate::settle_until(async || {
                let rows = rows_shown();
                !rows.is_empty() && rows.iter().all(|(title, _)| !title.is_empty())
            })
            .await,
            "the key map drew no rows"
        );
        let rows = rows_shown();
        let mut problems = Vec::new();
        let mut checked = 0;
        for command in GMAIL {
            let spec = registry::all()
                .find(|spec| spec.id == *command)
                .expect("a registered command");
            for (_, keys) in rows.iter().filter(|(title, _)| title == spec.title) {
                checked += 1;
                if *keys != bindings_of(*command) {
                    problems.push(format!(
                        "{}: the key map shows {keys:?}, the registry binds {:?}",
                        spec.title,
                        bindings_of(*command)
                    ));
                }
                for (retired, old) in RETIRED {
                    if retired == command && keys.iter().any(|key| key == old) {
                        problems.push(format!("{}: still shows the retired {old}", spec.title));
                    }
                }
            }
        }
        assert!(
            checked >= 30,
            "only {checked} Gmail rows were in the key map: {:?}",
            rows.iter().take(12).collect::<Vec<_>>()
        );
        assert!(
            problems.is_empty(),
            "the key map:\n  {}",
            problems.join("\n  ")
        );
    });
}

/// Ctrl+K's palette: type a command's name and its row shows its key.
pub fn the_palette_shows_the_gmail_keys_and_no_old_ones() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let world = World::new(Flavor::Plain, Surface::List).await;
        let window = &world.window;
        press_binding(window, "ctrl+k");
        let bar = window.bar().expect("the bar");
        assert!(
            crate::settle_until(async || bar.is_open()).await,
            "ctrl+k did not open the palette"
        );
        let offered = bar.commands();
        let mut problems = Vec::new();
        let mut checked = 0;
        for command in GMAIL {
            if !offered.contains(command) {
                continue;
            }
            let spec = registry::all()
                .find(|spec| spec.id == *command)
                .expect("a registered command");
            bar.set_text(&format!(">{}", spec.title));
            crate::settle_until(async || bar.texts().iter().any(|text| text == spec.title)).await;
            let texts = bar.texts();
            let Some(at) = texts.iter().position(|text| text == spec.title) else {
                problems.push(format!(
                    "{}: typing its name finds no row: {texts:?}",
                    spec.title
                ));
                continue;
            };
            checked += 1;
            let shown = texts.get(at + 1).cloned().unwrap_or_default();
            if shown != taught(*command) {
                problems.push(format!(
                    "{}: the palette shows {shown:?}, the registry's key is {:?}",
                    spec.title,
                    taught(*command)
                ));
            }
        }
        assert!(
            checked >= 25,
            "only {checked} Gmail commands were in the palette"
        );
        assert!(
            problems.is_empty(),
            "the palette:\n  {}",
            problems.join("\n  ")
        );
    });
}

/// Settings' Keyboard section lists each command with the key it runs on.
pub fn settings_keyboard_lists_the_gmail_keys() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, _path) = crate::settings::one_message_under("").await;
        support::deliver_with(&window, "comma", Mods::CONTROL_MASK);
        let dialog = crate::settings::settings_shown(&window)
            .await
            .expect("Settings opened");
        let (_, row) = crate::settings::section_rows(&dialog)
            .into_iter()
            .find(|(name, _)| name == "Keyboard")
            .expect("a Keyboard section");
        support::click(&window, &row, 1);
        let list = || support::texts(&support::only(&dialog, "postio-settings-keys-list"));
        assert!(crate::settle_until(async || list().len() > 100).await);
        let texts = list();
        let mut problems = Vec::new();
        let mut checked = 0;
        for command in GMAIL {
            let spec = registry::all()
                .find(|spec| spec.id == *command)
                .expect("a registered command");
            let Some(at) = texts.iter().position(|text| text == spec.title) else {
                continue;
            };
            checked += 1;
            // Settings writes the binding as the file does: `mod`, not `ctrl`.
            let shown = texts.get(at + 1).cloned().unwrap_or_default();
            if shown != spec.default_binding {
                problems.push(format!(
                    "{}: Settings shows {shown:?}, the registry's key is {:?}",
                    spec.title, spec.default_binding
                ));
            }
        }
        assert!(
            checked >= 30,
            "only {checked} Gmail commands were in Settings"
        );
        assert!(
            problems.is_empty(),
            "Settings:\n  {}",
            problems.join("\n  ")
        );
    });
}

// ---------------------------------------------------------------------------
// Where text is typed, Gmail's letters are letters
// ---------------------------------------------------------------------------

/// Gmail's keys are bare letters, so wherever text is being typed they must
/// type, not run: `e` in the composer's field is an `e`, not an archive.
pub fn gmail_letters_type_in_the_composer_and_in_the_search_bar() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        const LETTERS: &[&str] = &[
            "e", "r", "a", "f", "#", "s", "b", "v", "I", "U", "z", "o", "u", "j", "k", "n", "p",
            ";", "x", "T", "E", "V", "A", "N", "M", "Z", "*", "g",
        ];
        let world = World::new(Flavor::Plain, Surface::List).await;
        let window = &world.window;
        let watch = world.aimed();
        let mut problems = Vec::new();
        for (place, open) in [
            ("the composer", CommandId::Compose),
            ("the search bar", CommandId::Search),
        ] {
            world.reset().await;
            window.act(open);
            let typing = crate::settle_until(async || window.observe().keyboard.typing).await;
            assert!(typing, "{place}: the keyboard never went to a text field");
            crate::settle_for(std::time::Duration::from_millis(500)).await;
            let before = Snap::take(&world, watch).await;
            for key in LETTERS {
                press_binding(window, key);
            }
            crate::settle_for(std::time::Duration::from_millis(300)).await;
            let after = Snap::take(&world, watch).await;
            let mut changed = Vec::new();
            if after.mailbox() != before.mailbox() {
                changed.push("the message moved".to_owned());
            }
            if after.has(Flag::Flagged) != before.has(Flag::Flagged)
                || after.has(Flag::Seen) != before.has(Flag::Seen)
            {
                changed.push("its flags changed".to_owned());
            }
            if after.cursor() != before.cursor() || after.selected() != before.selected() {
                changed.push("the list's cursor or selection moved".to_owned());
            }
            if after.picker.is_some() || after.key_map || after.source || after.capture.is_some() {
                changed.push("something opened over the window".to_owned());
            }
            if after.place != before.place || after.places {
                changed.push("it went to another place".to_owned());
            }
            if after.reading_open {
                changed.push("a message opened".to_owned());
            }
            if open == CommandId::Compose && !after.compose {
                changed.push("the composer closed".to_owned());
            }
            if open == CommandId::Search && !after.bar {
                changed.push("the search bar closed".to_owned());
            }
            if !changed.is_empty() {
                problems.push(format!(
                    "{place}: typing the Gmail letters ran them: {changed:?}"
                ));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    });
}

// ---------------------------------------------------------------------------
// Words built elsewhere
// ---------------------------------------------------------------------------

/// The reader's pane for a message whose body is not here names the key that
/// checks for new mail; it must be one the registry gives Refresh.
pub fn the_reader_names_the_key_that_checks_for_new_mail() {
    use postio_ui::reader::document::{Absent, absent_html};
    let bound = bindings_of(CommandId::Refresh);
    let mut checked = 0;
    for state in [Absent::Offline, Absent::Missing] {
        let html = absent_html(state);
        let Some(named) = html
            .split("<kbd>")
            .nth(1)
            .and_then(|rest| rest.split("</kbd>").next())
        else {
            panic!("{state:?} names no key: {html}");
        };
        assert!(
            bound.iter().any(|key| key == named),
            "{state:?} says to press {named}; Refresh answers to {bound:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 2);
}

// ---------------------------------------------------------------------------
// Settings' account list
// ---------------------------------------------------------------------------

/// `z`, Gmail's undo, takes back a removed account where the list has the
/// keyboard, as `mod+z` always did.
pub fn z_takes_back_a_removed_account_in_settings() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        let (second, _) = fixture.second_account().await;
        let (window, _directory, _path) = crate::settings::open_under(&fixture, "").await;
        support::deliver_with(&window, "comma", Mods::CONTROL_MASK);
        let dialog = crate::settings::settings_shown(&window)
            .await
            .expect("Settings opened");
        let rows = || {
            support::with_class(&dialog, "postio-settings-account-row")
                .into_iter()
                .filter(|row| row.is_mapped())
                .filter_map(|row| row.downcast::<gtk::ListBoxRow>().ok())
                .collect::<Vec<_>>()
        };
        assert!(crate::settle_until(async || rows().len() == 2).await);
        let pending = async || {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::AccountRepository::new(&connection)
                .get(second.id)
                .await
                .expect("the account reads")
                .expect("still there")
                .pending_deletion
        };
        let mut reds = Vec::new();
        for undo in bindings_of(CommandId::Undo) {
            let name = format!("postio-account-{}", second.id.get());
            let row = rows()
                .into_iter()
                .find(|row| row.widget_name() == name.as_str())
                .expect("the account's row is on screen");
            row.grab_focus();
            crate::settle();
            support::deliver(&window, "Delete");
            assert!(
                crate::settle_until(async || pending().await).await,
                "Delete marked the account for removal"
            );
            // The removal's own toast would answer an Undo first; the row is
            // what has the keyboard, so put it back there.
            let row = rows()
                .into_iter()
                .find(|row| row.widget_name() == name.as_str());
            if let Some(row) = row {
                row.grab_focus();
            }
            crate::settle();
            press_binding(&window, &undo);
            if !crate::settle_until_within(std::time::Duration::from_secs(5), async || {
                !pending().await
            })
            .await
            {
                reds.push(format!("{undo} did not restore the removed account"));
                // Put it back so the next key starts from the same place.
                support::deliver_with(&window, "z", Mods::CONTROL_MASK);
                crate::settle_until(async || !pending().await).await;
            }
        }
        assert!(
            reds.is_empty(),
            "Settings' account list:\n  {}",
            reds.join("\n  ")
        );
    });
}
