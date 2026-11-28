use postio_body::{EditIntent, Unit};

use crate::keymap::Chord;

pub const EDITING_KEYS: &[(&str, KeyEdit)] = &[
    ("Return", KeyEdit::SplitBlock),
    ("shift+Return", KeyEdit::InsertBreak),
    ("BackSpace", KeyEdit::DeleteBackward(Unit::Char)),
    ("shift+BackSpace", KeyEdit::DeleteBackward(Unit::Char)),
    ("ctrl+BackSpace", KeyEdit::DeleteBackward(Unit::Word)),
    ("Delete", KeyEdit::DeleteForward(Unit::Char)),
    ("ctrl+Delete", KeyEdit::DeleteForward(Unit::Word)),
    ("Tab", KeyEdit::Indent),
    ("shift+Tab", KeyEdit::Outdent),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEdit {
    SplitBlock,
    InsertBreak,
    DeleteBackward(Unit),
    DeleteForward(Unit),
    Indent,
    Outdent,
}

impl KeyEdit {
    pub fn intent(self) -> EditIntent {
        match self {
            KeyEdit::SplitBlock => EditIntent::SplitBlock,
            KeyEdit::InsertBreak => EditIntent::InsertBreak,
            KeyEdit::DeleteBackward(unit) => EditIntent::DeleteBackward(unit),
            KeyEdit::DeleteForward(unit) => EditIntent::DeleteForward(unit),
            KeyEdit::Indent => EditIntent::Indent,
            KeyEdit::Outdent => EditIntent::Outdent,
        }
    }

    pub fn falls_through(self) -> bool {
        matches!(self, KeyEdit::Indent | KeyEdit::Outdent)
    }
}

pub fn key_edit(chord: &Chord) -> Option<KeyEdit> {
    EDITING_KEYS
        .iter()
        .find(|(written, _)| written.parse::<Chord>().is_ok_and(|key| key == *chord))
        .map(|(_, edit)| *edit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::{Context, registry};

    #[test]
    fn every_editing_key_is_a_chord_and_shadows_no_composer_command() {
        for (written, _) in EDITING_KEYS {
            let chord: Chord = written
                .parse()
                .unwrap_or_else(|_| panic!("{written} is not a chord"));
            assert!(
                registry::lookup_binding(Context::Composer, &chord.to_string()).is_none(),
                "{written} is a composer command's key; the surface must not take it"
            );
        }
    }

    #[test]
    fn a_key_finds_its_edit_and_a_command_key_finds_none() {
        let edit = |written: &str| key_edit(&written.parse().unwrap());
        assert_eq!(edit("Return"), Some(KeyEdit::SplitBlock));
        assert_eq!(edit("shift+Return"), Some(KeyEdit::InsertBreak));
        assert_eq!(
            edit("ctrl+BackSpace"),
            Some(KeyEdit::DeleteBackward(Unit::Word))
        );
        assert_eq!(edit("shift+Tab"), Some(KeyEdit::Outdent));
        assert_eq!(edit("ctrl+b"), None, "bold is the registry's");
        assert_eq!(edit("a"), None, "typing is text, not a key");
    }
}
