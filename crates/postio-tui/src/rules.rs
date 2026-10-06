//! The digest rules list's state (`g d`, terminal.md, "Digests"): every rule
//! in `config.toml`, what each holds now, the one the keyboard is on, and
//! the question over removing one.

use postio_config::DigestRule;

/// The question over `Delete`: remove this rule?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remove {
    /// The rule's name.
    pub name: String,
    /// What it holds now.
    pub holds: u32,
}

/// The list. See the module.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    rules: Vec<DigestRule>,
    holds: Vec<u32>,
    cursor: usize,
    top: usize,
    removing: Option<Remove>,
}

impl Rules {
    /// The list of `rules`, holding nothing yet.
    pub fn new(rules: Vec<DigestRule>) -> Rules {
        Rules {
            holds: vec![0; rules.len()],
            rules,
            ..Rules::default()
        }
    }

    /// The rules, in the file's order.
    pub fn rules(&self) -> &[DigestRule] {
        &self.rules
    }

    /// What each rule holds now, in the same order.
    pub fn holds(&self) -> &[u32] {
        &self.holds
    }

    /// The rules' names, to ask what each holds.
    pub fn names(&self) -> Vec<String> {
        self.rules.iter().map(|rule| rule.name.clone()).collect()
    }

    /// The rule the keyboard is on, with what it holds.
    pub fn focused(&self) -> Option<(&DigestRule, u32)> {
        Some((
            self.rules.get(self.cursor)?,
            self.holds.get(self.cursor).copied().unwrap_or(0),
        ))
    }

    /// The place the keyboard is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The first row in view.
    pub fn top(&self) -> usize {
        self.top
    }

    /// The question over removing a rule.
    pub fn removing(&self) -> Option<&Remove> {
        self.removing.as_ref()
    }

    /// Put the question up for the focused rule.
    pub fn ask_remove(&mut self) {
        self.removing = self.focused().map(|(rule, holds)| Remove {
            name: rule.name.clone(),
            holds,
        });
    }

    /// Take the question down.
    pub fn keep(&mut self) {
        self.removing = None;
    }

    /// What each of `names` holds arrived; kept only while the list still is
    /// the one asked about.
    pub fn held(&mut self, names: &[String], holds: Vec<u32>) {
        if self.names() == names && holds.len() == names.len() {
            self.holds = holds;
        }
    }

    /// The same rules read again, as `config.toml` has them now: the keyboard
    /// stays on its rule when it is still there.
    pub fn reread(&mut self, rules: Vec<DigestRule>) {
        let kept = self.focused().map(|(rule, _)| rule.name.clone());
        self.holds = vec![0; rules.len()];
        self.rules = rules;
        self.cursor = kept
            .and_then(|name| self.rules.iter().position(|rule| rule.name == name))
            .unwrap_or_else(|| self.cursor.min(self.rules.len().saturating_sub(1)));
    }

    /// Take the rule called `name` off the list.
    pub fn forget(&mut self, name: &str) {
        if let Some(at) = self.rules.iter().position(|rule| rule.name == name) {
            self.rules.remove(at);
            self.holds.remove(at);
        }
        self.cursor = self.cursor.min(self.rules.len().saturating_sub(1));
    }

    /// Move the keyboard by `by` rules.
    pub fn step(&mut self, by: isize) {
        let last = self.rules.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Move the keyboard to the rule at `at`, when there is one.
    pub fn go_to(&mut self, at: usize) {
        if at < self.rules.len() {
            self.cursor = at;
        }
    }

    /// Move the keyboard to the last rule.
    pub fn last(&mut self) {
        self.cursor = self.rules.len().saturating_sub(1);
    }

    /// Keep the rule the keyboard is on in view, in `height` rows.
    pub fn reveal(&mut self, height: usize) {
        let height = height.max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

    /// Scroll by `lines`, never past the last rule.
    pub fn scroll(&mut self, lines: isize, height: usize) {
        let last = self
            .rules
            .len()
            .saturating_sub(height.min(self.rules.len()));
        self.top = self.top.saturating_add_signed(lines).min(last);
        self.cursor = self.cursor.clamp(self.top, self.top + height.max(1) - 1);
        self.cursor = self.cursor.min(self.rules.len().saturating_sub(1));
    }
}
