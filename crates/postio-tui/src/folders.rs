//! The go-to-folder box over the controller's places: the filter, the row
//! the keyboard is on and where it goes back to. What is listed, in what
//! order and under which key is `FocusController::places`.

use crossterm::event::{KeyCode, KeyEvent};
use postio_core::CommandId;
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::places::{self as rules, Entry};
use tui_input::backend::crossterm::EventHandler;

use crate::app::Focus;
use crate::input::Keys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Stay,
    Close,
    /// The filter changed: list again.
    Filter,
    /// Go to the place the controller knows by this token.
    Go(u64),
    /// A view of its own, or a direct `g` key: run the command.
    Run(CommandId),
}

#[derive(Debug)]
pub struct Folders {
    filter: tui_input::Input,
    selected: usize,
    from: Focus,
    entries: Vec<(u64, Entry)>,
    /// A `g` the keymap has taken as the start of a chord, waiting for the
    /// key that finishes it.
    held: Option<KeyEvent>,
}

impl Folders {
    pub fn open(from: Focus, entries: Vec<(u64, Entry)>) -> Folders {
        Folders {
            filter: tui_input::Input::default(),
            selected: 0,
            from,
            entries,
            held: None,
        }
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn filter(&self) -> &str {
        self.filter.value()
    }

    pub fn caret(&self) -> usize {
        self.filter.cursor()
    }

    pub fn show(&mut self, entries: Vec<(u64, Entry)>) {
        self.entries = entries;
    }

    pub fn entries(&self) -> &[(u64, Entry)] {
        &self.entries
    }

    pub fn chosen(&self) -> usize {
        self.selected.min(self.entries.len().saturating_sub(1))
    }

    pub fn go(&self, index: usize) -> Step {
        match self.entries.get(index) {
            Some((
                _,
                Entry {
                    command: Some(command),
                    ..
                },
            )) => Step::Run(*command),
            Some((token, _)) => Step::Go(*token),
            None => Step::Stay,
        }
    }

    pub fn wheel(&mut self, down: bool) {
        let last = self.entries.len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 3).min(last)
        } else {
            self.selected.saturating_sub(3)
        };
    }

    /// With nothing typed, `g` and the key after it are the keymap's own:
    /// `g a` goes to the archive from here as from the list.
    pub fn key(&mut self, key: &KeyEvent, keys: &mut Keys) -> Step {
        let typing = !self.filter.value().is_empty();
        let outcome = keys.press(key, KeyContext::Search, typing);
        if let Outcome::Command(id) = &outcome
            && id == "back"
        {
            return Step::Close;
        }
        let mut step = Step::Stay;
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
                (Some(held), _) => step = self.type_in(&held),
                (None, _) => {}
            }
        }
        match key.code {
            KeyCode::Down => {
                self.selected = (self.chosen() + 1).min(self.entries.len().saturating_sub(1))
            }
            KeyCode::Up => self.selected = self.chosen().saturating_sub(1),
            KeyCode::Enter => return self.go(self.chosen()),
            _ => {
                if self.type_in(key) == Step::Filter {
                    step = Step::Filter;
                }
            }
        }
        step
    }

    fn type_in(&mut self, key: &KeyEvent) -> Step {
        let before = self.filter.value().to_owned();
        self.filter
            .handle_event(&crossterm::event::Event::Key(*key));
        if self.filter.value() == before {
            return Step::Stay;
        }
        self.selected = 0;
        Step::Filter
    }

    fn direct(id: &str) -> Option<CommandId> {
        let command: CommandId = id.parse().ok()?;
        // Filtered (`g f`), the digest rules (`g r`) and the scope cycle
        // (`g c`) are as reachable from here as `g i` is.
        (rules::direct_commands().contains(&command)
            || matches!(
                command,
                CommandId::GoToFiltered | CommandId::GoToDigestRules | CommandId::NextScope
            ))
        .then_some(command)
    }
}
