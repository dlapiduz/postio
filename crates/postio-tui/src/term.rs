//! Entering and leaving the terminal's modes, and always leaving them: every mode entered is
//! recorded, and leaving undoes exactly those in reverse order, on quit, on a panic and around
//! `$EDITOR`. Modes go through `Console`, so the order is testable against a recording.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Raw,
    AlternateScreen,
    Mouse,
    BracketedPaste,
    KeyboardEnhancement,
}

pub trait Console {
    /// Enter `mode` (`on`) or leave it.
    fn set(&mut self, mode: Mode, on: bool) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct Session {
    entered: Vec<Mode>,
}

impl Session {
    /// If one fails, the ones already entered are left again before the failure is returned.
    pub fn enter(console: &mut impl Console, modes: &[Mode]) -> io::Result<Session> {
        let mut session = Session::default();
        for mode in modes {
            if let Err(error) = console.set(*mode, true) {
                let _ = session.leave(console);
                return Err(error);
            }
            session.entered.push(*mode);
        }
        Ok(session)
    }

    /// Tries every mode even if one fails, and reports the first failure.
    pub fn leave(&mut self, console: &mut impl Console) -> io::Result<()> {
        let mut first = Ok(());
        while let Some(mode) = self.entered.pop() {
            if let Err(error) = console.set(mode, false)
                && first.is_ok()
            {
                first = Err(error);
            }
        }
        first
    }

    pub fn suspended<T>(
        &mut self,
        console: &mut impl Console,
        away: impl FnOnce() -> T,
    ) -> io::Result<T> {
        let modes = self.entered.clone();
        self.leave(console)?;
        let result = away();
        *self = Session::enter(console, &modes)?;
        Ok(result)
    }

    pub fn entered(&self) -> &[Mode] {
        &self.entered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recording(Vec<(Mode, bool)>);

    impl Console for Recording {
        fn set(&mut self, mode: Mode, on: bool) -> io::Result<()> {
            self.0.push((mode, on));
            Ok(())
        }
    }

    const ALL: [Mode; 5] = [
        Mode::Raw,
        Mode::AlternateScreen,
        Mode::Mouse,
        Mode::BracketedPaste,
        Mode::KeyboardEnhancement,
    ];

    #[test]
    fn leaving_undoes_every_mode_entered_in_reverse() {
        let mut console = Recording::default();
        let mut session = Session::enter(&mut console, &ALL).expect("entered");
        session.leave(&mut console).expect("left");

        let entered: Vec<_> = ALL.iter().map(|mode| (*mode, true)).collect();
        let left: Vec<_> = ALL.iter().rev().map(|mode| (*mode, false)).collect();
        assert_eq!(console.0, [entered, left].concat());
        assert!(session.entered().is_empty());
    }

    #[test]
    fn a_mode_that_would_not_turn_on_is_not_turned_off() {
        struct NoMouse(Recording);
        impl Console for NoMouse {
            fn set(&mut self, mode: Mode, on: bool) -> io::Result<()> {
                if mode == Mode::Mouse && on {
                    return Err(io::Error::other("no mouse here"));
                }
                self.0.set(mode, on)
            }
        }
        let mut console = NoMouse(Recording::default());
        let session = Session::enter(&mut console, &ALL);
        assert!(session.is_err());
        assert!(
            !console.0.0.contains(&(Mode::Mouse, false)),
            "never on, never turned off: {:?}",
            console.0.0
        );
    }

    #[test]
    fn the_editor_gets_a_normal_terminal_and_the_frontend_gets_its_modes_back() {
        let mut console = Recording::default();
        let mut session = Session::enter(&mut console, &ALL).expect("entered");
        console.0.clear();

        let during = session
            .suspended(&mut console, || "the editor ran")
            .expect("handed back");

        assert_eq!(during, "the editor ran");
        let left: Vec<_> = ALL.iter().rev().map(|mode| (*mode, false)).collect();
        let entered: Vec<_> = ALL.iter().map(|mode| (*mode, true)).collect();
        assert_eq!(console.0, [left, entered].concat());
        assert_eq!(session.entered(), ALL);
    }
}

#[derive(Debug, Default)]
pub struct Stdout;

impl Console for Stdout {
    fn set(&mut self, mode: Mode, on: bool) -> io::Result<()> {
        use crossterm::event::{
            DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        };
        use crossterm::execute;
        use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
        let mut out = io::stdout();
        match (mode, on) {
            (Mode::Raw, true) => crossterm::terminal::enable_raw_mode(),
            (Mode::Raw, false) => crossterm::terminal::disable_raw_mode(),
            (Mode::AlternateScreen, true) => execute!(out, EnterAlternateScreen),
            (Mode::AlternateScreen, false) => execute!(out, LeaveAlternateScreen),
            (Mode::Mouse, true) => execute!(out, EnableMouseCapture),
            (Mode::Mouse, false) => execute!(out, DisableMouseCapture),
            (Mode::BracketedPaste, true) => execute!(out, EnableBracketedPaste),
            (Mode::BracketedPaste, false) => execute!(out, DisableBracketedPaste),
            // Enough to tell ctrl+Return from Return and ctrl+shift+x from
            // ctrl+x, which is what the registry's defaults need (research R4).
            (Mode::KeyboardEnhancement, true) => execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            ),
            (Mode::KeyboardEnhancement, false) => execute!(out, PopKeyboardEnhancementFlags),
        }
    }
}

/// What is entered now, for the panic hook, which has no session.
static ENTERED: std::sync::Mutex<Vec<Mode>> = std::sync::Mutex::new(Vec::new());

impl Session {
    pub fn has(&self, mode: Mode) -> bool {
        self.entered.contains(&mode)
    }

    /// Called by the loop that owns the real terminal after every change of modes.
    pub fn publish(&self) {
        *ENTERED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.entered.clone();
    }
}

/// Restores the terminal before a panic's message prints, and before `panic = "abort"` ends the
/// process with the terminal raw.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let entered = std::mem::take(
            &mut *ENTERED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let _ = Session { entered }.leave(&mut Stdout);
        previous(info);
    }));
}
