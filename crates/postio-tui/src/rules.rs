//! The digest rules list's state: each rule, the one the keyboard is on, and the remove question.

use postio_config::DigestRule;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remove {
    pub name: String,
    pub holds: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    rules: Vec<DigestRule>,
    holds: Vec<u32>,
    cursor: usize,
    top: usize,
    removing: Option<Remove>,
}

impl Rules {
    pub fn new(rules: Vec<DigestRule>) -> Rules {
        Rules {
            holds: vec![0; rules.len()],
            rules,
            ..Rules::default()
        }
    }

    pub fn rules(&self) -> &[DigestRule] {
        &self.rules
    }

    pub fn holds(&self) -> &[u32] {
        &self.holds
    }

    pub fn names(&self) -> Vec<String> {
        self.rules.iter().map(|rule| rule.name.clone()).collect()
    }

    pub fn focused(&self) -> Option<(&DigestRule, u32)> {
        Some((
            self.rules.get(self.cursor)?,
            self.holds.get(self.cursor).copied().unwrap_or(0),
        ))
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn removing(&self) -> Option<&Remove> {
        self.removing.as_ref()
    }

    pub fn ask_remove(&mut self) {
        self.removing = self.focused().map(|(rule, holds)| Remove {
            name: rule.name.clone(),
            holds,
        });
    }

    pub fn keep(&mut self) {
        self.removing = None;
    }

    pub fn held(&mut self, names: &[String], holds: Vec<u32>) {
        if self.names() == names && holds.len() == names.len() {
            self.holds = holds;
        }
    }

    pub fn reread(&mut self, rules: Vec<DigestRule>) {
        let kept = self.focused().map(|(rule, _)| rule.name.clone());
        self.holds = vec![0; rules.len()];
        self.rules = rules;
        self.cursor = kept
            .and_then(|name| self.rules.iter().position(|rule| rule.name == name))
            .unwrap_or_else(|| self.cursor.min(self.rules.len().saturating_sub(1)));
    }

    pub fn forget(&mut self, name: &str) {
        if let Some(at) = self.rules.iter().position(|rule| rule.name == name) {
            self.rules.remove(at);
            self.holds.remove(at);
        }
        self.cursor = self.cursor.min(self.rules.len().saturating_sub(1));
    }

    pub fn step(&mut self, by: isize) {
        let last = self.rules.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    pub fn go_to(&mut self, at: usize) {
        if at < self.rules.len() {
            self.cursor = at;
        }
    }

    pub fn last(&mut self) {
        self.cursor = self.rules.len().saturating_sub(1);
    }

    pub fn reveal(&mut self, height: usize) {
        let height = height.max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

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
