//! The rule dialog as drawn: the controller's view, and what only the
//! keyboard knows: the control it is on and the two fields typed into.

use postio_focus::RuleView;
use postio_model::listing::Cadence;
use postio_ui::digest::{self, Schedule};
use postio_ui::terminal::SafeText;
use tui_input::Input;

use crate::app::Focus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Cadence,
    Day,
    Time,
    Query,
    MatchInstead,
    LikeThis,
    Create,
}

#[derive(Debug, Clone)]
pub struct Form {
    view: RuleView,
    query: Input,
    time: Input,
    field: Field,
    from: Focus,
}

impl Form {
    pub fn new(view: RuleView, from: Focus) -> Form {
        let mut form = Form {
            query: Input::default(),
            time: Input::default(),
            field: Field::Cadence,
            from,
            view,
        };
        if form.view.query.is_some() {
            form.field = Field::Query;
        }
        form.sync();
        form
    }

    pub fn view(&self) -> &RuleView {
        &self.view
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn query(&self) -> &Input {
        &self.query
    }

    pub fn time(&self) -> &Input {
        &self.time
    }

    pub fn field(&self) -> Field {
        self.field
    }

    pub fn schedule(&self) -> &Schedule {
        &self.view.schedule
    }

    pub fn replace(&mut self, view: RuleView) {
        let matching = view.query.is_some() && self.view.query.is_none();
        self.view = view;
        if matching {
            self.field = Field::Query;
        }
        if !self.fields().contains(&self.field) {
            self.field = Field::Create;
        }
        self.sync();
    }

    // Words the controller holds replace the fields' only when they differ,
    // so a caret is not moved by the echo of its own typing.
    fn sync(&mut self) {
        let query = SafeText::new(self.view.query.as_deref().unwrap_or_default());
        if self.query.value() != query.as_str() {
            self.query = Input::new(query.as_str().to_owned());
        }
        if self.time.value() != self.view.schedule.at {
            self.time = Input::new(self.view.schedule.at.clone());
        }
    }

    pub fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Cadence];
        if self.view.schedule.cadence_kind() != Cadence::Daily {
            fields.push(Field::Day);
        }
        fields.push(Field::Time);
        if self.view.query.is_some() {
            fields.push(Field::Query);
        } else {
            fields.push(Field::MatchInstead);
            if self.view.like_this.is_some() {
                fields.push(Field::LikeThis);
            }
        }
        fields.push(Field::Create);
        fields
    }

    pub fn typing(&self) -> bool {
        matches!(self.field, Field::Time | Field::Query)
    }

    pub fn walk(&mut self, by: isize) {
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = (at as isize + by).rem_euclid(fields.len() as isize) as usize;
        self.field = fields[next];
    }

    pub fn focus(&mut self, field: Field) {
        if self.fields().contains(&field) {
            self.field = field;
        }
    }

    pub fn changed(&self, by: isize) -> Option<Schedule> {
        let mut schedule = self.view.schedule.clone();
        match self.field {
            Field::Cadence => {
                let at = schedule.cadence as isize + by;
                schedule.cadence = at.rem_euclid(digest::CADENCES.len() as isize) as usize;
            }
            Field::Day if schedule.cadence_kind() == Cadence::Weekly => {
                let at = schedule.weekday as isize + by;
                schedule.weekday = at.rem_euclid(digest::WEEKDAYS.len() as isize) as usize;
            }
            Field::Day => {
                let at = schedule.month_day as isize + by;
                schedule.month_day = at.rem_euclid(28) as usize;
            }
            _ => return None,
        }
        Some(schedule)
    }

    pub fn input(&mut self) -> Option<&mut Input> {
        match self.field {
            Field::Time => Some(&mut self.time),
            Field::Query => Some(&mut self.query),
            _ => None,
        }
    }
}
