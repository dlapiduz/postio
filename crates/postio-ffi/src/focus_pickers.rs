//! Focus's pickers at the row at the boundary (specs/009-focus-macos T089,
//! for the Mac's T091-T092).
//!
//! The pickers are the controller's (`postio_focus`, ADR 0045): which opens,
//! what it acts on, what it lists and what each row does. The Mac draws the
//! picker it is told (`FocusOpenPicker`, then `FocusPickerRows` for every
//! change), keeps the arrows and the highlight, and reports what happened:
//! the field's text ([`Session::focus_picker_typed`]), a row chosen
//! ([`Session::focus_picker_choose`]) or toggled
//! ([`Session::focus_picker_toggle`]). The number keys, `Tab`, Return with
//! nothing highlighted, and Esc are commands `key` resolves in the picker's
//! context and `invoke` runs (`picker_choose_1`-`4`, `picker_type_date`,
//! `picker_confirm`, `back`). The picker closes through the surface stack
//! like every other surface (`FocusCloseSurface { kind: Picker }`, and
//! `focus_surface_closed(Picker)` when it closes any other way).

use crate::session::Session;

/// Which picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PickerKindFfi {
    /// "Snooze until" (`s`, screen 11).
    Snooze,
    /// "Remind me if no one replies by" (`h`, screen 12).
    Remind,
    /// "Labels" (`l`, screen 13).
    Label,
    /// "Move to folder" (`m`, screen 14).
    Move,
}

impl From<postio_focus::PickerKind> for PickerKindFfi {
    fn from(kind: postio_focus::PickerKind) -> Self {
        use postio_focus::PickerKind as Kind;
        match kind {
            Kind::Snooze => PickerKindFfi::Snooze,
            Kind::Remind => PickerKindFfi::Remind,
            Kind::Label => PickerKindFfi::Label,
            Kind::Move => PickerKindFfi::Move,
        }
    }
}

/// What a picker hangs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PickerAnchorFfi {
    /// The list's row at `position`: the cursor's. Hang it under the row,
    /// at the subject column.
    Row {
        /// The row.
        position: u32,
    },
    /// The open message's window, under its action row.
    OpenMessage,
    /// A search result's row at `position`: the focus ring's (spec 010
    /// US5). Hang it under the row, as from the list's.
    Result {
        /// The row.
        position: u64,
    },
}

impl From<postio_focus::Anchor> for PickerAnchorFfi {
    fn from(anchor: postio_focus::Anchor) -> Self {
        match anchor {
            postio_focus::Anchor::Row(position) => PickerAnchorFfi::Row { position },
            postio_focus::Anchor::OpenMessage => PickerAnchorFfi::OpenMessage,
            postio_focus::Anchor::Result(position) => PickerAnchorFfi::Result { position },
        }
    }
}

/// The field a picker holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PickerFieldFfi {
    /// A date typed in words, under the rows. It takes the keyboard on
    /// `FocusPickerField` (`Tab`); until then the number keys are the
    /// picker's.
    Date,
    /// A filter over the rows, above them, holding the keyboard from the
    /// start.
    Filter,
}

impl From<postio_focus::PickerField> for PickerFieldFfi {
    fn from(field: postio_focus::PickerField) -> Self {
        match field {
            postio_focus::PickerField::Date => PickerFieldFfi::Date,
            postio_focus::PickerField::Filter => PickerFieldFfi::Filter,
        }
    }
}

/// One row of a picker.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PickerRowFfi {
    /// What [`Session::focus_picker_choose`] and
    /// [`Session::focus_picker_toggle`] take. Never reused: a token from
    /// rows since redrawn does nothing.
    pub token: u64,
    /// The heading of the section this row starts, when it starts one:
    /// "Recent", "All folders".
    pub section: Option<String>,
    /// The name, in bold.
    pub name: String,
    /// On the right: a time, a count, "✓ applied".
    pub detail: String,
    /// The number key that chooses it, as the keymap spells it ("1"); the
    /// keycap drawn on the row.
    pub key: Option<String>,
    /// Whether a label's colour dot is drawn before the name.
    pub dot: bool,
    /// The label's stored colour, `#rrggbb`; `None` for one the frontend
    /// picks from the name, as the places popover's dots.
    pub color: Option<String>,
    /// Whether the label is on everything the picker acts on.
    pub applied: bool,
    /// Whether it is "Create label “…”".
    pub create: bool,
}

impl From<postio_focus::PickerRow> for PickerRowFfi {
    fn from(row: postio_focus::PickerRow) -> Self {
        PickerRowFfi {
            token: row.token,
            section: row.section,
            name: row.name,
            detail: row.detail,
            key: row.key,
            dot: row.dot,
            color: row.color,
            applied: row.applied,
            create: row.create,
        }
    }
}

/// A picker, whole: everything `FocusOpenPicker` and `FocusPickerRows`
/// draw.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PickerViewFfi {
    /// Which picker.
    pub kind: PickerKindFfi,
    /// What it hangs from.
    pub anchor: PickerAnchorFfi,
    /// Its title: "Snooze until".
    pub title: String,
    /// What it acts on: "Ada Moreno · Atlas Q3 budget", "3 conversations".
    pub target: String,
    /// The field it holds.
    pub field: PickerFieldFfi,
    /// What the field says while it is empty.
    pub placeholder: String,
    /// What the field holds: set it from this only when it differs, since
    /// [`Session::focus_picker_typed`] hears every change.
    pub typed: String,
    /// The date field's line under it: "Tab to type", when the words land
    /// ("Tue 29 Sep, 09:00"), or what it wants. `None` for a filter.
    pub hint: Option<String>,
    /// The rows, top to bottom. A label or move picker opens with none, and
    /// `FocusPickerRows` brings them once read.
    pub rows: Vec<PickerRowFfi>,
    /// The footnote: what happens next, with its keys.
    pub footnote: String,
}

impl From<postio_focus::PickerView> for PickerViewFfi {
    fn from(view: postio_focus::PickerView) -> Self {
        PickerViewFfi {
            kind: view.kind.into(),
            anchor: view.anchor.into(),
            title: view.title,
            target: view.target,
            field: view.field.into(),
            placeholder: view.placeholder,
            typed: view.typed,
            hint: view.hint,
            rows: view.rows.into_iter().map(PickerRowFfi::from).collect(),
            footnote: view.footnote,
        }
    }
}

#[uniffi::export]
impl Session {
    /// The picker's field holds `text` now: the date typed, or the filter.
    /// Said on every change; the same text again changes nothing.
    pub fn focus_picker_typed(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::PickerTyped { text });
    }

    /// The picker's row `token` was chosen: a click on it, or Return while
    /// it is highlighted. A preset or folder acts and the picker closes; a
    /// label goes on or off and the picker stays.
    pub fn focus_picker_choose(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::PickerChoose(token));
    }

    /// `Space` on the picker's highlighted row `token`: a label on or off,
    /// or the label "Create label" names made and put on. The picker stays.
    pub fn focus_picker_toggle(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::PickerToggle(token));
    }
}
