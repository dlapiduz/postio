//! The terminal frontend's state, and the one function that changes it.
//!
//! `update` takes an [`Input`] and returns the [`Effect`]s it asks for --
//! a request of the daemon, a redraw, quitting -- and does no I/O itself.
//! That is what lets every behaviour be driven by synthetic input in a test,
//! with no terminal and no daemon (research R11).

use crate::layout::{self, Requested, Shown};

/// Something that happened, from the terminal or from the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
}

/// Something `update` asks the loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Draw a frame.
    Redraw,
    /// Leave.
    Quit,
}

/// Everything the terminal frontend knows.
#[derive(Debug, Clone)]
pub struct App {
    size: (u16, u16),
    requested: Requested,
}

impl App {
    /// A frontend in a terminal of `size`.
    pub fn new(size: (u16, u16)) -> App {
        App {
            size,
            requested: Requested::default(),
        }
    }

    /// What the user asked to see.
    pub fn requested(&self) -> Requested {
        self.requested
    }

    /// What the terminal has room for.
    pub fn shown(&self) -> Shown {
        layout::shown(self.size.0, self.size.1, self.requested)
    }
}

/// Take in one input; say what should happen next.
pub fn update(app: &mut App, input: Input) -> Vec<Effect> {
    match input {
        Input::Resize(width, height) => {
            app.size = (width, height);
            vec![Effect::Redraw]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Pane;

    #[test]
    fn a_resize_changes_what_is_shown_and_asks_the_daemon_nothing() {
        let mut app = App::new((160, 40));
        let asked = app.requested();

        let effects = update(&mut app, Input::Resize(60, 30));

        assert_eq!(effects, vec![Effect::Redraw], "a redraw and nothing else");
        assert_eq!(app.shown(), Shown::Panes(vec![Pane::List]));
        assert_eq!(app.requested(), asked, "what was asked for is untouched");

        update(&mut app, Input::Resize(160, 40));
        assert_eq!(
            app.shown(),
            Shown::Panes(vec![Pane::Sidebar, Pane::List, Pane::Reader]),
            "widening brings the sidebar back"
        );
    }
}
