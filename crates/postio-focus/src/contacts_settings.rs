//! Settings > Contacts' Suggestions switches: the pane asks when it opens, a click asks the host
//! to turn exactly that switch over, and the pane is redrawn from the answer.

use postio_config::paths::Platform;
use postio_model::{ContactSuggestions, Suggestion};
use postio_ui::contacts_settings::{Device, SuggestionsView, suggestions};

use crate::feed::Step;
use crate::{FocusController, Intent, Request};

#[derive(Debug, Clone, Default)]
pub(crate) struct Settings {
    switches: Option<ContactSuggestions>,
    error: Option<String>,
}

impl FocusController {
    fn contacts_settings_view(&self) -> Option<SuggestionsView> {
        let settings = self.contacts_settings.as_ref()?;
        let switches = settings.switches?;
        let apple = self.policy.platform == Platform::Apple;
        let mut view = suggestions(
            &switches,
            if apple { Device::Mac } else { Device::Computer },
            apple,
        );
        view.error = settings.error.clone();
        Some(view)
    }

    pub(crate) fn contacts_settings_opened(&mut self) -> Vec<Step> {
        self.contacts_settings.get_or_insert_with(Settings::default);
        vec![Step::Ask(Request::ContactSuggestions)]
    }

    pub(crate) fn contacts_suggestion_clicked(&mut self, index: u32) -> Vec<Step> {
        let Some(switches) = self
            .contacts_settings
            .as_ref()
            .and_then(|settings| settings.switches)
        else {
            return Vec::new();
        };
        let Some(which) = Suggestion::ALL.get(index as usize).copied() else {
            return Vec::new();
        };
        vec![Step::Ask(Request::ContactSuggestionSet {
            which,
            on: !switches.is_on(which),
        })]
    }

    pub(crate) fn contacts_suggestions_read(
        &mut self,
        answer: Result<ContactSuggestions, String>,
    ) -> Vec<Step> {
        let Some(settings) = self.contacts_settings.as_mut() else {
            return Vec::new();
        };
        match answer {
            Ok(switches) => {
                settings.switches = Some(switches);
                settings.error = None;
            }
            Err(reason) => settings.error = Some(reason),
        }
        self.contacts_settings_view()
            .map(|view| vec![Step::Show(Intent::ContactsSettings(Box::new(view)))])
            .unwrap_or_default()
    }

    pub fn contacts_settings(&self) -> Option<SuggestionsView> {
        self.contacts_settings_view()
    }
}
