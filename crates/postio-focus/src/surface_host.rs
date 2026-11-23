use postio_core::CommandId;

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, SurfaceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Window {
    #[default]
    Main,
    Calendar,
    Composer,
}

impl FocusController {
    pub(crate) fn calendar_popped(&self) -> bool {
        self.calendar
            .as_ref()
            .is_some_and(|calendar| calendar.popped)
    }

    pub(crate) fn pop_out(&mut self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        if calendar.popped {
            return Vec::new();
        }
        calendar.popped = true;
        self.surfaces.dismiss(SurfaceKind::Calendar);
        self.focused = Window::Calendar;
        let mut steps = vec![
            Step::Show(Intent::CloseSurface(SurfaceKind::Calendar)),
            Step::Show(Intent::CalendarWindow(true)),
        ];
        steps.extend(self.draw_calendar());
        steps.push(Step::Show(Intent::PresentWindow(Window::Calendar)));
        steps
    }

    pub(crate) fn pop_in(&mut self) -> Vec<Step> {
        let Some(calendar) = self.calendar.as_mut() else {
            return Vec::new();
        };
        if !calendar.popped {
            return Vec::new();
        }
        calendar.popped = false;
        self.focused = Window::Main;
        let mut steps = vec![Step::Show(Intent::CalendarWindow(false))];
        steps.extend(
            self.surfaces
                .opened(SurfaceKind::Calendar, self.policy.caps.stacking),
        );
        steps.extend(self.draw_calendar());
        steps.push(Step::Show(Intent::PresentWindow(Window::Main)));
        steps
    }

    pub(crate) fn calendar_window_command(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Step> {
        match id {
            CommandId::PopIn | CommandId::CloseWindow => self.pop_in(),
            CommandId::GoToCalendar => Vec::new(),
            CommandId::Back if self.calendar_at_root() => Vec::new(),
            id if crate::bar::goes(id) || crate::calendar::is_destination(id) => {
                self.focused = Window::Main;
                let mut steps = vec![Step::Show(Intent::PresentWindow(Window::Main))];
                steps.extend(self.list_steps(id, rows));
                steps
            }
            id => self.calendar_command(id, rows).unwrap_or_default(),
        }
    }

    pub(crate) fn close_window(&mut self, rows: &dyn Rows) -> Vec<Step> {
        match self.focused {
            Window::Calendar if self.calendar_popped() => self.pop_in(),
            Window::Composer if self.surfaces.has(SurfaceKind::Composer) => {
                self.focused = Window::Main;
                self.composer_command(CommandId::Back, rows)
            }
            _ => vec![Step::Show(Intent::Quit)],
        }
    }
}
