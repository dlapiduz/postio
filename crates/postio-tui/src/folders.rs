//! The folders popover (terminal.md, "Folders and labels"): `g o`, or a
//! click on the strip's place, lists the places to go -- mailboxes with
//! their direct keys, the person's folders, and labels -- with their counts.
//! Typing filters them, and `Enter` goes to the one chosen.
//!
//! What is listed, how it sorts and which key goes straight to each is
//! `postio_ui::places`'; this holds the filter, the chosen row and the keys.

use crossterm::event::{KeyCode, KeyEvent};
use postio_core::CommandId;
use postio_model::Mailbox;
use postio_ui::finder::Destination;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::places::{self as rules, Entry};
use tui_input::backend::crossterm::EventHandler;

use crate::app::Focus;
use crate::input::Keys;
use crate::places::PlaceDetails;

/// What the app knows that the list is built from.
pub struct Reach<'a> {
    /// Every folder of every account.
    pub folders: &'a [Mailbox],
    /// How many were filtered today, while filtering is on.
    pub filtered_today: Option<u32>,
}

/// What a key or a click asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Draw again.
    Stay,
    /// Put the popover away.
    Close,
    /// Go to this place.
    Go(Destination, String),
    /// Run this command: Filtered's, or a direct `g` key.
    Run(CommandId),
}

/// The popover, open.
#[derive(Debug)]
pub struct Folders {
    filter: tui_input::Input,
    selected: usize,
    from: Focus,
    details: PlaceDetails,
    /// A `g` the keymap has taken as the start of a chord, waiting for the
    /// key that finishes it.
    held: Option<KeyEvent>,
}

impl Folders {
    /// A popover open over `from`.
    pub fn open(from: Focus) -> Folders {
        Folders {
            filter: tui_input::Input::default(),
            selected: 0,
            from,
            details: PlaceDetails::default(),
            held: None,
        }
    }

    /// Where the keyboard was when it opened.
    pub fn from(&self) -> Focus {
        self.from
    }

    /// What is typed.
    pub fn filter(&self) -> &str {
        self.filter.value()
    }

    /// Where the caret is, in characters.
    pub fn caret(&self) -> usize {
        self.filter.cursor()
    }

    /// The labels, counts and Outbox the host read.
    pub fn learn(&mut self, details: PlaceDetails) {
        self.details = details;
    }

    /// The places the filter keeps, in their sections.
    pub fn entries(&self, reach: &Reach<'_>) -> Vec<Entry> {
        let mut all: Vec<Entry> = reach
            .folders
            .iter()
            .filter(|folder| folder.selectable)
            .map(rules::mailbox_entry)
            .collect();
        for (account, waiting) in &self.details.outbox {
            if *waiting > 0 {
                all.push(rules::outbox_entry(*account, *waiting));
            }
        }
        for label in &self.details.labels {
            let mut entry = rules::label_entry(label);
            entry.count = self
                .details
                .label_counts
                .iter()
                .find(|(id, _)| *id == label.id)
                .map(|(_, count)| count.to_string());
            all.push(entry);
        }
        let filtered = reach.filtered_today.map(rules::filtered_entry);
        rules::listed(&all, filtered.as_ref(), self.filter.value())
    }

    /// The chosen row, as an index into the entries.
    pub fn chosen(&self, entries: &[Entry]) -> usize {
        self.selected.min(entries.len().saturating_sub(1))
    }

    /// Go to the `index`th place.
    pub fn go(&self, index: usize, reach: &Reach<'_>) -> Step {
        match self.entries(reach).into_iter().nth(index) {
            Some(entry) => match entry.command {
                Some(command) => Step::Run(command),
                None => Step::Go(entry.destination, entry.name),
            },
            None => Step::Stay,
        }
    }

    /// The wheel: the chosen row moves.
    pub fn wheel(&mut self, down: bool, reach: &Reach<'_>) {
        let last = self.entries(reach).len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 3).min(last)
        } else {
            self.selected.saturating_sub(3)
        };
    }

    /// A key in the popover. With nothing typed, `g` and the key after it
    /// are the keymap's own: `g r` goes to the archive from here as from the
    /// list.
    pub fn key(&mut self, key: &KeyEvent, keys: &mut Keys, reach: &Reach<'_>) -> Step {
        // With nothing typed the keymap's own keys are heard; once there is
        // a filter every key is typing, but for the one that closes.
        let typing = !self.filter.value().is_empty();
        let outcome = keys.press(key, KeyContext::Search, typing);
        if let Outcome::Command(id) = &outcome
            && id == "back"
        {
            return Step::Close;
        }
        if !typing {
            match (self.held.take(), outcome) {
                (_, Outcome::Command(id)) if Self::direct(&id).is_some() => {
                    return Step::Run(Self::direct(&id).expect("checked"));
                }
                (None, Outcome::Pending(_)) if key.modifiers.is_empty() => {
                    self.held = Some(*key);
                    return Step::Stay;
                }
                // The key after a `g` that finished nothing: both are typed.
                (Some(held), _) => self.type_in(&held),
                (None, _) => {}
            }
        }
        match key.code {
            KeyCode::Down => {
                let last = self.entries(reach).len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Enter => {
                let entries = self.entries(reach);
                return self.go(self.chosen(&entries), reach);
            }
            _ => self.type_in(key),
        }
        Step::Stay
    }

    fn type_in(&mut self, key: &KeyEvent) {
        let before = self.filter.value().to_owned();
        self.filter
            .handle_event(&crossterm::event::Event::Key(*key));
        if self.filter.value() != before {
            self.selected = 0;
        }
    }

    /// The go-to command a name is, when it is one with a key of its own.
    fn direct(id: &str) -> Option<CommandId> {
        let command: CommandId = id.parse().ok()?;
        rules::direct_commands()
            .contains(&command)
            .then_some(command)
    }
}
