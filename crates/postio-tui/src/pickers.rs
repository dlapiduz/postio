//! The picker box over the controller's `PickerView`: the caret, the row the keyboard is on and
//! where it goes back to.

use postio_focus::{PickerField, PickerKind, PickerRow, PickerView};
use postio_ui::terminal::SafeText;

use crate::app::Focus;

#[derive(Debug)]
pub struct Picker {
    view: PickerView,
    from: Focus,
    field: tui_input::Input,
    in_field: bool,
    selected: usize,
}

impl Picker {
    pub fn opened(mut view: PickerView, from: Focus) -> Picker {
        clean(&mut view);
        Picker {
            in_field: view.field == PickerField::Filter,
            view,
            from,
            field: tui_input::Input::default(),
            selected: 0,
        }
    }

    pub fn show(&mut self, mut view: PickerView) {
        clean(&mut view);
        if view.typed != self.view.typed {
            self.selected = 0;
        }
        self.view = view;
    }

    pub fn view(&self) -> &PickerView {
        &self.view
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn kind(&self) -> PickerKind {
        self.view.kind
    }

    pub fn typed(&self) -> &str {
        self.field.value()
    }

    pub fn caret(&self) -> usize {
        self.field.cursor()
    }

    pub fn in_field(&self) -> bool {
        self.in_field
    }

    pub fn into_field(&mut self) {
        self.in_field = true;
    }

    pub fn field_mut(&mut self) -> &mut tui_input::Input {
        &mut self.field
    }

    pub fn chosen(&self) -> usize {
        self.selected.min(self.view.rows.len().saturating_sub(1))
    }

    pub fn row(&self, index: usize) -> Option<&PickerRow> {
        self.view.rows.get(index)
    }

    pub fn step(&mut self, down: bool, by: usize) {
        let last = self.view.rows.len().saturating_sub(1);
        self.selected = if down {
            (self.chosen() + by).min(last)
        } else {
            self.chosen().saturating_sub(by)
        };
    }

    pub fn chosen_row(&self) -> Option<&PickerRow> {
        self.row(self.chosen())
    }
}

fn clean(view: &mut PickerView) {
    for row in &mut view.rows {
        row.name = SafeText::new(&row.name).as_str().to_owned();
    }
}
