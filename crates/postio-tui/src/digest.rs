//! The digest window as drawn: the controller's view and this terminal's scroll.

use postio_focus::DigestView;
use postio_ui::digest::Page;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    view: DigestView,
    top: usize,
}

impl Window {
    pub fn new(view: DigestView) -> Window {
        Window { view, top: 0 }
    }

    pub fn view(&self) -> &DigestView {
        &self.view
    }

    pub fn page(&self) -> Page {
        self.view.page
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn scroll_to(&mut self, top: usize) {
        self.top = top;
    }

    pub fn cursor(&self) -> Option<usize> {
        self.view.focused.map(|at| at as usize)
    }

    pub fn reference(&self) -> Option<usize> {
        self.view.focused_reference.map(|at| at as usize)
    }

    pub fn replace(&mut self, view: DigestView) {
        let moved = view.page != self.view.page
            || view.email.as_ref().map(|email| email.message)
                != self.view.email.as_ref().map(|email| email.message);
        self.view = view;
        if moved {
            self.top = 0;
        }
    }
}
