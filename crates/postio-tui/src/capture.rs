//! The capture sheet as drawn: the controller's view, and what only the
//! keyboard knows: the control it is on, the text being typed and the
//! project chooser's row.

use postio_focus::CaptureView;
use postio_ui::terminal::SafeText;
use tui_input::Input;

use crate::app::Focus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Text,
    Due,
    Project,
}

#[derive(Debug, Clone)]
pub struct Sheet {
    view: CaptureView,
    text: Input,
    filter: Input,
    field: Field,
    at: usize,
    from: Focus,
}

impl Sheet {
    pub fn new(view: CaptureView, from: Focus) -> Sheet {
        let mut sheet = Sheet {
            text: Input::default(),
            filter: Input::default(),
            field: Field::Text,
            at: 0,
            from,
            view,
        };
        sheet.sync();
        sheet
    }

    pub fn view(&self) -> &CaptureView {
        &self.view
    }

    pub fn mode(&self) -> postio_ui::capture::Mode {
        self.view.mode
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn text(&self) -> &Input {
        &self.text
    }

    pub fn text_mut(&mut self) -> &mut Input {
        &mut self.text
    }

    pub fn filter(&self) -> &Input {
        &self.filter
    }

    pub fn filter_mut(&mut self) -> &mut Input {
        &mut self.filter
    }

    pub fn field(&self) -> Field {
        self.field
    }

    pub fn at(&self) -> usize {
        self.at
    }

    pub fn choosing(&self) -> bool {
        self.view.projects_open
    }

    pub fn typing(&self) -> bool {
        self.choosing() || self.field == Field::Text
    }

    pub fn replace(&mut self, view: CaptureView) {
        let opened = view.projects_open && !self.view.projects_open;
        let closed = !view.projects_open && self.view.projects_open;
        self.view = view;
        if opened {
            self.field = Field::Project;
            self.at = 0;
        }
        if closed {
            self.field = Field::Text;
        }
        if !self.view.has_due && self.field == Field::Due {
            self.field = Field::Text;
        }
        self.sync();
    }

    // Words the controller holds replace the fields' only when they differ,
    // so a caret is not moved by the echo of its own typing.
    fn sync(&mut self) {
        let text = SafeText::new(&self.view.text);
        if self.text.value() != text.as_str() {
            self.text = Input::new(text.as_str().to_owned());
        }
        if self.filter.value() != self.view.filter {
            self.filter = Input::new(self.view.filter.clone());
        }
        self.at = self.at.min(self.view.projects.len().saturating_sub(1));
    }

    pub fn focus(&mut self, field: Field) {
        if field != Field::Due || self.view.has_due {
            self.field = field;
        }
    }

    pub fn walk(&mut self, by: isize) {
        let fields: Vec<Field> = [Field::Text, Field::Due, Field::Project]
            .into_iter()
            .filter(|field| *field != Field::Due || self.view.has_due)
            .collect();
        let at = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = (at as isize + by).rem_euclid(fields.len() as isize) as usize;
        self.field = fields[next];
    }

    pub fn choose_by(&mut self, by: isize) {
        let last = self.view.projects.len().saturating_sub(1);
        self.at = self.at.saturating_add_signed(by).min(last);
    }

    pub fn pick_by(&self, by: isize) -> Option<Option<chrono::NaiveDate>> {
        let picks = &self.view.picks;
        let at = picks
            .iter()
            .position(|pick| pick.day == self.view.due)
            .unwrap_or(picks.len().saturating_sub(1));
        let next = at
            .saturating_add_signed(by)
            .min(picks.len().checked_sub(1)?);
        Some(picks[next].day)
    }
}
