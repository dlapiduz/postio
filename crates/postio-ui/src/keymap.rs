//! The keymap resolver: single keys, chords, sequences and contexts, with no widget in it (not GTK
//! accelerators). A `g g` sequence needs a visible pending state, `Esc` means different things per
//! context, and typing must always win over a single-key binding; none of that fits
//! `gtk::ShortcutController`.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use postio_core::{Context, ContextSet, registry};

/// Long enough to type deliberately, short enough that a leftover `g` never swallows the next
/// keystroke.
pub const CHORD_TIMEOUT: Duration = Duration::from_millis(1_000);

/// `Shift` is not one for printable keys: the character already says it (see `Chord`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const SHIFT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::CTRL),
            "alt" => Some(Self::ALT),
            "shift" => Some(Self::SHIFT),
            // `cmd` is Apple's ⌘, the same modifier X11 calls Super. `expand_mod` resolves `mod` to
            // `cmd` on Apple (#669), so without it every `mod+…` default fails to parse there.
            "super" | "meta" | "cmd" | "command" => Some(Self::SUPER),
            _ => None,
        }
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, name) in [
            (Self::CTRL, "ctrl"),
            (Self::ALT, "alt"),
            (Self::SHIFT, "shift"),
            (Self::SUPER, "super"),
        ] {
            if self.contains(bit) {
                write!(f, "{name}+")?;
            }
        }
        Ok(())
    }
}

/// The left column is every accepted spelling; the right is the one stored, so `esc` and `Escape`
/// are one key.
const NAMED_KEYS: &[(&str, &str)] = &[
    ("return", "Return"),
    ("enter", "Return"),
    ("escape", "Escape"),
    ("esc", "Escape"),
    ("tab", "Tab"),
    ("space", "Space"),
    ("backspace", "BackSpace"),
    ("delete", "Delete"),
    ("insert", "Insert"),
    ("home", "Home"),
    ("end", "End"),
    ("page_up", "Page_Up"),
    ("pageup", "Page_Up"),
    ("prior", "Page_Up"),
    ("page_down", "Page_Down"),
    ("pagedown", "Page_Down"),
    ("next", "Page_Down"),
    ("up", "Up"),
    ("down", "Down"),
    ("left", "Left"),
    ("right", "Right"),
    ("menu", "Menu"),
    ("kp_add", "KP_Add"),
    ("kp_subtract", "KP_Subtract"),
    ("kp_0", "KP_0"),
    ("f1", "F1"),
    ("f2", "F2"),
    ("f3", "F3"),
    ("f4", "F4"),
    ("f5", "F5"),
    ("f6", "F6"),
    ("f7", "F7"),
    ("f8", "F8"),
    ("f9", "F9"),
    ("f10", "F10"),
    ("f11", "F11"),
    ("f12", "F12"),
];

/// `?` and `question` must be the same chord, or a binding copied from a GDK key table never
/// matches.
const PUNCTUATION_NAMES: &[(&str, char)] = &[
    ("exclam", '!'),
    ("plus", '+'),
    ("minus", '-'),
    ("equal", '='),
    ("slash", '/'),
    ("backslash", '\\'),
    ("question", '?'),
    ("comma", ','),
    ("period", '.'),
    ("semicolon", ';'),
    ("colon", ':'),
    ("asterisk", '*'),
    ("numbersign", '#'),
    ("underscore", '_'),
    ("less", '<'),
    ("greater", '>'),
    ("bracketleft", '['),
    ("bracketright", ']'),
    ("braceleft", '{'),
    ("braceright", '}'),
    ("grave", '`'),
    ("apostrophe", '\''),
    ("quotedbl", '"'),
    // Strikethrough's and the table's terminal keys (spec 014).
    ("asciitilde", '~'),
    ("bar", '|'),
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Char(char),
    Named(&'static str),
}

impl Key {
    /// Only `Escape` and the function keys do; characters, `Return`, `Tab`, arrows and `Home`/`End`
    /// belong to the text field.
    fn survives_text_entry(&self) -> bool {
        match self {
            Self::Char(_) => false,
            Self::Named(name) => {
                *name == "Escape" || (name.starts_with('F') && name[1..].parse::<u8>().is_ok())
            }
        }
    }

    /// The keysym name (`question` for `?`), as X11 and `gdk::Key::from_name` accept; the
    /// storyboard runner uses it to press a chord.
    pub fn keysym_name(&self) -> String {
        match self {
            Self::Char(character) => PUNCTUATION_NAMES
                .iter()
                .find(|(_, aliased)| aliased == character)
                .map_or_else(|| character.to_string(), |(name, _)| (*name).to_owned()),
            Self::Named("Space") => "space".to_owned(),
            Self::Named(name) => (*name).to_owned(),
        }
    }

    fn parse(name: &str) -> Option<Self> {
        let mut characters = name.chars();
        if let (Some(character), None) = (characters.next(), characters.next()) {
            return Some(Self::Char(character));
        }
        let lower = name.to_ascii_lowercase();
        if let Some((_, character)) = PUNCTUATION_NAMES.iter().find(|(alias, _)| *alias == lower) {
            return Some(Self::Char(*character));
        }
        NAMED_KEYS
            .iter()
            .find(|(alias, _)| *alias == lower)
            .map(|(_, canonical)| Self::Named(canonical))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(character) => write!(f, "{character}"),
            Self::Named(name) => f.write_str(name),
        }
    }
}

/// Stored as the keyboard delivers it: for a character key `Shift` is folded into the character
/// (`shift+a` is `A`); for a named key it stays a modifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Chord {
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        match key {
            Key::Char(character) if modifiers.contains(Modifiers::SHIFT) => {
                let upper = character.to_uppercase().next().unwrap_or(character);
                Self {
                    key: Key::Char(upper),
                    modifiers: modifiers.without(Modifiers::SHIFT),
                }
            }
            key => Self { key, modifiers },
        }
    }

    /// From the character the key types, its name when it types nothing (GDK reports `Return` as
    /// `\r`), and the modifiers. `None` for a key with no name here. Deliberately not a key code:
    /// GTK and AppKit disagree on those.
    pub fn from_platform_key(
        character: Option<char>,
        name: Option<&str>,
        modifiers: Modifiers,
    ) -> Option<Self> {
        if let Some(character) = character {
            if character == ' ' {
                return Some(Self::new(Key::Named("Space"), modifiers));
            }
            if !character.is_control() {
                return Some(Self::new(Key::Char(character), modifiers));
            }
        }
        Key::parse(name?).map(|key| Self::new(key, modifiers))
    }

    /// True with `Ctrl`, `Alt` or `Super`, or for a key a text field would not want; this keeps `a`
    /// from archiving while typing.
    pub fn survives_text_entry(&self) -> bool {
        // Shift alone does not make a shortcut: `shift+a` is still a letter to a text field.
        if !self.modifiers.without(Modifiers::SHIFT).is_empty() {
            return true;
        }
        self.key.survives_text_entry()
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.modifiers, self.key)
    }
}

impl FromStr for Chord {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, ParseError> {
        if text.is_empty() {
            return Err(ParseError::Empty);
        }
        // A lone `+` is the plus key, not an empty modifier list.
        if text == "+" {
            return Ok(Self::new(Key::Char('+'), Modifiers::NONE));
        }

        let parts: Vec<&str> = text.split('+').collect();
        let (key, modifier_names) = parts.split_last().expect("split yields at least one part");

        let mut modifiers = Modifiers::NONE;
        for name in modifier_names {
            if name.eq_ignore_ascii_case("mod") {
                return Err(ParseError::UnexpandedMod);
            }
            let parsed = Modifiers::parse(name).ok_or_else(|| ParseError::UnknownModifier {
                modifier: (*name).to_owned(),
            })?;
            modifiers = modifiers.with(parsed);
        }

        if key.is_empty() {
            return Err(ParseError::NoKey {
                chord: text.to_owned(),
            });
        }
        let key = Key::parse(key).ok_or_else(|| ParseError::UnknownKey {
            key: (*key).to_owned(),
        })?;
        Ok(Self::new(key, modifiers))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Binding(Vec<Chord>);

impl Binding {
    pub fn chords(&self) -> &[Chord] {
        &self.0
    }

    pub fn first(&self) -> &Chord {
        &self.0[0]
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    fn starts_with(&self, prefix: &[Chord]) -> bool {
        self.0.len() > prefix.len() && self.0.starts_with(prefix)
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, chord) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{chord}")?;
        }
        Ok(())
    }
}

impl FromStr for Binding {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, ParseError> {
        let chords = text
            .split_whitespace()
            .map(Chord::from_str)
            .collect::<Result<Vec<_>, _>>()?;
        if chords.is_empty() {
            return Err(ParseError::Empty);
        }
        Ok(Self(chords))
    }
}

/// Shown beside the `[keys]` entry it came from: an unparseable binding is a settings problem, not
/// a crash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("a binding cannot be empty")]
    Empty,
    /// Its own variant so the message does not send anyone hunting for a typo: `mod` is a
    /// config word that `Keymap::resolve` expands, and code reading a registry or `[keys]` literal
    /// must expand it first.
    #[error(
        "`mod` is resolved when the keymap is built, so it cannot reach the \
         resolver; expand it with `postio_config::keys::expand_mod` first"
    )]
    UnexpandedMod,

    #[error("`{modifier}` is not a modifier; use mod, ctrl, alt, shift or super")]
    UnknownModifier { modifier: String },
    #[error("`{chord}` ends with a modifier and no key")]
    NoKey { chord: String },
    #[error("`{key}` is not a key name")]
    UnknownKey { key: String },
}

/// Contexts fall back rather than replace: reading a message still leaves the list's `j`/`k`
/// working. The chain is `KeyContext::chain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum KeyContext {
    #[default]
    Global,
    List,
    /// The whole thread stacked beside the list; its keys walk messages inside the stack.
    Conversation,
    Reader,
    Composer,
    Search,
    Palette,
    Sidebar,
    Parts,
    Accounts,
    Keys,
    Picker,
    Digest,
    Filtered,
    Capture,
    Results,
    Contacts,
    ContactPopover,
    Outline,
    ForwardChooser,
    Source,
}

impl KeyContext {
    /// The precedence order is this chain. Composer, Search and Palette do not fall back through
    /// the list: with the composer open, `a` is the letter A, not archive.
    pub fn chain(self) -> &'static [KeyContext] {
        match self {
            Self::Global => &[Self::Global],
            Self::List => &[Self::List, Self::Global],
            Self::Conversation => &[Self::Conversation, Self::List, Self::Global],
            Self::Reader => &[Self::Reader, Self::Conversation, Self::List, Self::Global],
            Self::Composer => &[Self::Composer, Self::Global],
            Self::Search => &[Self::Search, Self::Global],
            Self::Palette => &[Self::Palette, Self::Global],
            // Not layered over `List`: `j` means "next folder" here and "next message" there.
            Self::Sidebar => &[Self::Sidebar, Self::Global],
            // Not layered: the panel sits over a message, and `a` falling through would act on it.
            Self::Parts => &[Self::Parts, Self::Global],
            // Not layered: `d` here removes an account, so a fall-through mail binding would act on
            // something off-screen.
            Self::Accounts => &[Self::Accounts, Self::Global],
            // As `Accounts`: a fall-through would let a mail binding fire on a rebind row.
            Self::Keys => &[Self::Keys, Self::Global],
            // A picker sits over a row, so `x` or `a` falling through would act on the mail under
            // it;
            // digest and Filtered are surfaces of their own. Each takes its own keys, plus `Escape`
            // and the palette from Global.
            Self::Picker => &[Self::Picker, Self::Global],
            Self::Digest => &[Self::Digest, Self::Global],
            Self::Filtered => &[Self::Filtered, Self::Global],
            // Sits over a message like a picker: a letter falling through would act on the mail
            // under it.
            Self::Capture => &[Self::Capture, Self::Global],
            // Not layered over `List`: a list key falling through would act on the inbox hidden
            // behind the results.
            Self::Results => &[Self::Results, Self::Global],
            // Not layered over `List`: `c` here writes to the focused person, and a mail binding
            // would act on the list behind.
            Self::Contacts => &[Self::Contacts, Self::Global],
            Self::ContactPopover => &[Self::ContactPopover, Self::Global],
            // `u` is `prev_view` in the reader and "first unread" here, and the digits are the
            // chooser's. Neither falls through.
            Self::Outline => &[Self::Outline, Self::Global],
            Self::ForwardChooser => &[Self::ForwardChooser, Self::Composer, Self::Global],
            Self::Source => &[Self::Source, Self::Global],
        }
    }
}

impl From<Context> for KeyContext {
    /// One-to-one. `Global` has no counterpart: core spells "everywhere" as `ContextSet::ANY`,
    /// collapsed in `Keymap::from_commands`.
    fn from(context: Context) -> Self {
        match context {
            Context::List => Self::List,
            Context::Conversation => Self::Conversation,
            Context::Reader => Self::Reader,
            Context::Composer => Self::Composer,
            Context::Search => Self::Search,
            Context::Palette => Self::Palette,
            Context::Sidebar => Self::Sidebar,
            Context::Parts => Self::Parts,
            Context::Accounts => Self::Accounts,
            Context::Keys => Self::Keys,
            Context::Picker => Self::Picker,
            Context::Digest => Self::Digest,
            Context::Filtered => Self::Filtered,
            Context::Capture => Self::Capture,
            Context::Results => Self::Results,
            Context::Contacts => Self::Contacts,
            Context::ContactPopover => Self::ContactPopover,
            Context::Outline => Self::Outline,
            Context::ForwardChooser => Self::ForwardChooser,
            Context::Source => Self::Source,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    entries: Vec<(KeyContext, Binding, String)>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebinding the same `(context, binding)` replaces it, so a `[keys]` override wins over the
    /// default.
    pub fn bind(
        &mut self,
        context: KeyContext,
        binding: &str,
        command: &str,
    ) -> Result<(), ParseError> {
        let binding: Binding = binding.parse()?;
        self.entries
            .retain(|(existing, bound, _)| !(*existing == context && *bound == binding));
        self.entries.push((context, binding, command.to_owned()));
        Ok(())
    }

    pub fn binding_for(&self, context: KeyContext, command: &str) -> Option<&Binding> {
        self.entries
            .iter()
            .find(|(existing, _, bound)| *existing == context && bound == command)
            .map(|(_, binding, _)| binding)
    }

    pub fn entries(&self) -> impl Iterator<Item = (KeyContext, &Binding, &str)> {
        self.entries
            .iter()
            .map(|(context, binding, command)| (*context, binding, command.as_str()))
    }

    /// Not an error at bind time: a user override can create one, and refusing to start over it
    /// would be worse. The settings panel shows these.
    pub fn shadowed(&self) -> Vec<(KeyContext, &Binding, &str)> {
        self.entries
            .iter()
            .filter(|(context, binding, _)| {
                self.entries.iter().any(|(other_context, other, _)| {
                    other_context == context && binding.starts_with(other.chords())
                })
            })
            .map(|(context, binding, command)| (*context, binding, command.as_str()))
            .collect()
    }

    /// `commands` has already applied `[keys]` overrides and dropped collisions. A command
    /// reachable in every context is bound once in `Global`.
    /// Also returns what could not be parsed: an unreadable binding costs its command a key and is
    /// reported, never stopping the app.
    pub fn from_commands(commands: &postio_core::Keymap) -> (Self, Vec<String>) {
        Self::build(commands, None)
    }

    /// Binds only the commands `frontend` offers: a command only another app offers keeps its key
    /// there, and here the key does what an unbound key does.
    pub fn from_commands_for(
        commands: &postio_core::Keymap,
        frontend: postio_core::Frontend,
    ) -> (Self, Vec<String>) {
        Self::build(commands, Some(frontend))
    }

    fn build(
        commands: &postio_core::Keymap,
        frontend: Option<postio_core::Frontend>,
    ) -> (Self, Vec<String>) {
        let mut keymap = Self::new();
        let mut problems = Vec::new();

        for spec in registry::every_action() {
            if frontend.is_some_and(|app| !spec.requires.offered_by(app)) {
                continue;
            }
            for binding in commands.bindings(spec.id) {
                let contexts: Vec<KeyContext> = if spec.contexts == ContextSet::ANY {
                    vec![KeyContext::Global]
                } else {
                    spec.contexts.iter().map(KeyContext::from).collect()
                };
                for context in contexts {
                    if let Err(error) = keymap.bind(context, binding, spec.id.as_str()) {
                        problems.push(format!(
                            "`{binding}` cannot be used for `{}`: {error}",
                            spec.id
                        ));
                    }
                }
            }
        }

        (keymap, problems)
    }

    fn lookup(&self, context: KeyContext, pressed: &[Chord], in_text_entry: bool) -> Match<'_> {
        let mut prefix = false;
        for candidate in context.chain() {
            for (entry_context, binding, command) in &self.entries {
                if entry_context != candidate {
                    continue;
                }
                if in_text_entry && !binding.first().survives_text_entry() {
                    continue;
                }
                if binding.chords() == pressed {
                    // An exact match wins at once: waiting for a timeout on a fully typed key reads
                    // as lag; `shadowed` reports the arrangement.
                    return Match::Exact(command.as_str());
                }
                if binding.starts_with(pressed) {
                    prefix = true;
                }
            }
            if prefix {
                // A more specific context has claimed this prefix; a fallback must not resolve it
                // from under the sequence.
                return Match::Prefix;
            }
        }
        Match::None
    }
}

enum Match<'a> {
    Exact(&'a str),
    Prefix,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Command(String),
    /// The string is what to show while pending, so the state is never invisible.
    Pending(String),
    Unhandled,
}

#[derive(Debug)]
pub struct Resolver {
    keymap: Keymap,
    timeout: Duration,
    pending: Vec<Chord>,
    expires_at: Option<Instant>,
}

impl Resolver {
    pub fn new(keymap: Keymap) -> Self {
        Self {
            keymap,
            timeout: CHORD_TIMEOUT,
            pending: Vec::new(),
            expires_at: None,
        }
    }

    pub fn from_commands(commands: &postio_core::Keymap) -> (Self, Vec<String>) {
        let (keymap, problems) = Keymap::from_commands(commands);
        let problems = Self::all_problems(commands, &keymap, problems);
        (Self::new(keymap), problems)
    }

    pub fn from_commands_for(
        commands: &postio_core::Keymap,
        frontend: postio_core::Frontend,
    ) -> (Self, Vec<String>) {
        let (keymap, problems) = Keymap::from_commands_for(commands, frontend);
        let problems = Self::all_problems(commands, &keymap, problems);
        (Self::new(keymap), problems)
    }

    /// `commands.problems()`, this crate's parse failures, then shadowed bindings, in that order.
    /// `keymap` must be the resolved table: shadowing is a property of it.
    fn all_problems(
        commands: &postio_core::Keymap,
        keymap: &Keymap,
        mut parse_problems: Vec<String>,
    ) -> Vec<String> {
        let mut problems = commands.problems().to_vec();
        problems.append(&mut parse_problems);
        problems.extend(
            keymap
                .shadowed()
                .into_iter()
                .map(|(context, binding, command)| {
                    format!(
                        "`{binding}` in {context:?} can never fire -- a shorter binding \
                 matches first, so `{command}` is unreachable"
                    )
                }),
        );
        problems
    }

    pub fn with_timeout(keymap: Keymap, timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::new(keymap)
        }
    }

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Any half-typed sequence is dropped: it was typed against the old table.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
        self.clear_pending();
    }

    pub fn pending(&self) -> Option<String> {
        (!self.pending.is_empty()).then(|| Self::describe(&self.pending))
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.expires_at = None;
    }

    /// While `in_text_entry`, a binding whose first chord is a bare character or editing key does
    /// not fire: typing wins.
    /// `now` is passed in so the timeout is testable without sleeping.
    pub fn press(
        &mut self,
        chord: &Chord,
        context: KeyContext,
        in_text_entry: bool,
        now: Instant,
    ) -> Outcome {
        if self.expires_at.is_some_and(|deadline| now >= deadline) {
            self.clear_pending();
        }

        let mut attempt = std::mem::take(&mut self.pending);
        attempt.push(chord.clone());

        match self.keymap.lookup(context, &attempt, in_text_entry) {
            Match::Exact(command) => {
                let command = command.to_owned();
                self.clear_pending();
                Outcome::Command(command)
            }
            Match::Prefix => {
                let description = Self::describe(&attempt);
                self.pending = attempt;
                self.expires_at = now.checked_add(self.timeout);
                Outcome::Pending(description)
            }
            Match::None => {
                // An abandoned sequence is not retried as a fresh first chord: `g` then `q` must
                // not run what `q` is bound to.
                self.clear_pending();
                Outcome::Unhandled
            }
        }
    }

    fn describe(chords: &[Chord]) -> String {
        chords
            .iter()
            .map(Chord::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        text.parse().expect("a chord")
    }

    fn canvas_keymap() -> Keymap {
        let mut keymap = Keymap::new();
        for (binding, command) in [
            ("ctrl+k", "command_palette"),
            ("?", "cheat_sheet"),
            ("ctrl+e", "edit_config"),
            ("c", "compose"),
            ("/", "search"),
            ("u", "undo"),
        ] {
            keymap.bind(KeyContext::Global, binding, command).unwrap();
        }
        for (binding, command) in [
            ("j", "next_message"),
            ("k", "prev_message"),
            ("Return", "open_message"),
            ("Escape", "clear_selection"),
            ("t", "thread"),
            ("a", "archive"),
            ("A", "archive_thread"),
            ("e", "reply"),
            ("E", "reply_all"),
            ("f", "forward"),
            ("g g", "go_to_top"),
            ("G", "go_to_bottom"),
        ] {
            keymap.bind(KeyContext::List, binding, command).unwrap();
        }
        keymap
            .bind(KeyContext::Conversation, "Escape", "back")
            .unwrap();
        keymap
            .bind(KeyContext::Composer, "Escape", "close_composer")
            .unwrap();
        keymap
            .bind(KeyContext::Composer, "ctrl+Return", "send")
            .unwrap();
        keymap
            .bind(KeyContext::Search, "Escape", "close_search")
            .unwrap();
        keymap
            .bind(KeyContext::Palette, "Escape", "close_palette")
            .unwrap();
        keymap
    }

    fn resolver() -> Resolver {
        Resolver::new(canvas_keymap())
    }

    fn press(resolver: &mut Resolver, keys: &str, context: KeyContext) -> Outcome {
        let now = Instant::now();
        let mut outcome = Outcome::Unhandled;
        for key in keys.split_whitespace() {
            outcome = resolver.press(&chord(key), context, false, now);
        }
        outcome
    }

    fn command(resolver: &mut Resolver, keys: &str, context: KeyContext) -> String {
        match press(resolver, keys, context) {
            Outcome::Command(command) => command,
            other => panic!("{keys} in {context:?} resolved to {other:?}"),
        }
    }

    #[test]
    fn a_shifted_character_is_the_same_chord_as_its_uppercase() {
        assert_eq!(chord("shift+a"), chord("A"));
        assert_eq!(chord("ctrl+shift+p"), chord("ctrl+P"));
        assert_eq!(
            chord("A").modifiers,
            Modifiers::NONE,
            "the character carries the shift, so the modifier must not linger"
        );
        assert_ne!(chord("a"), chord("A"), "the canvas binds these separately");
    }

    #[test]
    fn shift_stays_a_modifier_on_a_key_with_no_character() {
        let shifted = chord("shift+Tab");

        assert_eq!(shifted.key, Key::Named("Tab"));
        assert!(shifted.modifiers.contains(Modifiers::SHIFT));
        assert_ne!(shifted, chord("Tab"));
    }

    #[test]
    fn a_key_name_and_the_character_it_produces_are_one_chord() {
        assert_eq!(chord("question"), chord("?"));
        assert_eq!(chord("slash"), chord("/"));
        assert_eq!(chord("+"), chord("plus"), "a lone + is the plus key");
    }

    #[test]
    fn key_names_are_spelled_however_the_user_spelled_them() {
        assert_eq!(chord("esc"), chord("Escape"));
        assert_eq!(chord("escape"), chord("ESCAPE"));
        assert_eq!(chord("enter"), chord("Return"));
        assert_eq!(chord("pageup"), chord("page_up"));
        assert_eq!(
            chord("Escape").to_string(),
            "Escape",
            "displayed canonically"
        );
    }

    #[test]
    fn modifiers_are_spelled_however_the_user_spelled_them() {
        assert_eq!(chord("control+k"), chord("ctrl+k"));
        assert_eq!(chord("meta+k"), chord("super+k"));
    }

    #[test]
    fn a_broken_binding_says_what_is_wrong_with_it() {
        assert_eq!("".parse::<Binding>(), Err(ParseError::Empty));
        assert_eq!("   ".parse::<Binding>(), Err(ParseError::Empty));
        assert!(matches!(
            "ctrl+".parse::<Binding>(),
            Err(ParseError::NoKey { .. })
        ));
        assert!(matches!(
            "hyper+a".parse::<Binding>(),
            Err(ParseError::UnknownModifier { .. })
        ));
        assert!(matches!(
            "Retrun".parse::<Binding>(),
            Err(ParseError::UnknownKey { .. })
        ));
        assert!(matches!(
            "j ctrl+".parse::<Binding>(),
            Err(ParseError::NoKey { .. })
        ));
    }

    #[test]
    fn a_binding_displays_as_something_the_cheat_sheet_can_print() {
        assert_eq!("g g".parse::<Binding>().unwrap().to_string(), "g g");
        assert_eq!("ctrl+k".parse::<Binding>().unwrap().to_string(), "ctrl+k");
        assert_eq!(
            "shift+Tab".parse::<Binding>().unwrap().to_string(),
            "shift+Tab"
        );
    }

    #[test]
    fn the_canvas_bindings_resolve_in_the_list() {
        let mut resolver = resolver();

        for (keys, expected) in [
            ("j", "next_message"),
            ("k", "prev_message"),
            ("Return", "open_message"),
            ("t", "thread"),
            ("a", "archive"),
            ("A", "archive_thread"),
            ("e", "reply"),
            ("E", "reply_all"),
            ("f", "forward"),
            ("u", "undo"),
            ("c", "compose"),
            ("/", "search"),
            ("?", "cheat_sheet"),
            ("ctrl+k", "command_palette"),
        ] {
            assert_eq!(command(&mut resolver, keys, KeyContext::List), expected);
        }
    }

    #[test]
    fn a_lowercase_and_uppercase_binding_are_different_commands() {
        let mut resolver = resolver();

        assert_eq!(command(&mut resolver, "a", KeyContext::List), "archive");
        assert_eq!(
            command(&mut resolver, "A", KeyContext::List),
            "archive_thread",
            "which is what shift+a produces"
        );
        assert_eq!(
            command(&mut resolver, "shift+a", KeyContext::List),
            "archive_thread"
        );
    }

    #[test]
    fn escape_means_something_different_in_every_context() {
        let mut resolver = resolver();

        for (context, expected) in [
            (KeyContext::List, "clear_selection"),
            (KeyContext::Conversation, "back"),
            (KeyContext::Composer, "close_composer"),
            (KeyContext::Search, "close_search"),
            (KeyContext::Palette, "close_palette"),
        ] {
            assert_eq!(command(&mut resolver, "Escape", context), expected);
        }
    }

    #[test]
    fn the_reader_still_walks_the_list() {
        let mut resolver = resolver();

        assert_eq!(
            command(&mut resolver, "j", KeyContext::Reader),
            "next_message",
            "the canvas keeps j/k live while reading"
        );
        assert_eq!(
            command(&mut resolver, "Escape", KeyContext::Reader),
            "back",
            "the thread's Escape, not the list's, is the nearer one"
        );
    }

    #[test]
    fn the_composer_does_not_fall_back_through_the_list() {
        let mut resolver = resolver();

        assert_eq!(
            press(&mut resolver, "a", KeyContext::Composer),
            Outcome::Unhandled,
            "`a` is the letter A while composing, never archive"
        );
        assert_eq!(
            command(&mut resolver, "ctrl+k", KeyContext::Composer),
            "command_palette",
            "but global bindings still reach it"
        );
    }

    #[test]
    fn an_unbound_key_is_left_for_the_widget() {
        let mut resolver = resolver();

        assert_eq!(
            press(&mut resolver, "z", KeyContext::List),
            Outcome::Unhandled
        );
    }

    #[test]
    fn gg_and_g_uppercase_are_different_bindings() {
        let mut resolver = resolver();

        assert_eq!(command(&mut resolver, "g g", KeyContext::List), "go_to_top");
        assert_eq!(
            command(&mut resolver, "G", KeyContext::List),
            "go_to_bottom",
            "one press, not two"
        );
    }

    #[test]
    fn a_half_typed_sequence_is_visibly_pending() {
        let mut resolver = resolver();
        let now = Instant::now();

        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, false, now),
            Outcome::Pending("g".to_owned())
        );
        assert_eq!(resolver.pending().as_deref(), Some("g"));

        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, false, now),
            Outcome::Command("go_to_top".to_owned())
        );
        assert_eq!(resolver.pending(), None, "and it stops being pending");
    }

    #[test]
    fn a_sequence_that_goes_nowhere_is_abandoned_whole() {
        let mut resolver = resolver();
        let now = Instant::now();

        resolver.press(&chord("g"), KeyContext::List, false, now);
        assert_eq!(
            resolver.press(&chord("a"), KeyContext::List, false, now),
            Outcome::Unhandled,
            "`g a` must not fall through to archive"
        );
        assert_eq!(resolver.pending(), None);

        assert_eq!(
            command(&mut resolver, "a", KeyContext::List),
            "archive",
            "and the next press starts clean"
        );
    }

    #[test]
    fn a_pending_sequence_expires() {
        let mut resolver = Resolver::with_timeout(canvas_keymap(), Duration::from_millis(500));
        let start = Instant::now();

        resolver.press(&chord("g"), KeyContext::List, false, start);
        let later = start + Duration::from_millis(600);

        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, false, later),
            Outcome::Pending("g".to_owned()),
            "the first g was forgotten, so this one starts a new sequence"
        );
    }

    #[test]
    fn a_pending_sequence_survives_inside_the_timeout() {
        let mut resolver = Resolver::with_timeout(canvas_keymap(), Duration::from_millis(500));
        let start = Instant::now();

        resolver.press(&chord("g"), KeyContext::List, false, start);
        let later = start + Duration::from_millis(499);

        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, false, later),
            Outcome::Command("go_to_top".to_owned())
        );
    }

    #[test]
    fn losing_focus_forgets_a_half_typed_sequence() {
        let mut resolver = resolver();
        let now = Instant::now();

        resolver.press(&chord("g"), KeyContext::List, false, now);
        resolver.clear_pending();

        assert_eq!(resolver.pending(), None);
        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, false, now),
            Outcome::Pending("g".to_owned())
        );
    }

    #[test]
    fn a_shorter_binding_shadows_a_longer_one_and_says_so() {
        let mut keymap = canvas_keymap();
        keymap.bind(KeyContext::List, "g", "go_somewhere").unwrap();
        let shadowed: Vec<&str> = keymap
            .shadowed()
            .into_iter()
            .map(|(_, _, command)| command)
            .collect();

        assert_eq!(shadowed, vec!["go_to_top"]);

        let mut resolver = Resolver::new(keymap);
        assert_eq!(
            command(&mut resolver, "g", KeyContext::List),
            "go_somewhere",
            "the exact match fires rather than waiting for a timeout"
        );
    }

    #[test]
    fn a_shadowed_binding_reaches_the_problems_the_settings_panel_shows() {
        // Rebinding another command to plain `g` shadows `g g` (FirstMessage); the problem must
        // surface through `Resolver::from_commands`.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_string(), "g".to_string());

        let (_, problems) = Resolver::from_commands(&postio_core::Keymap::resolve(&overrides));

        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("first_message") && problem.contains("`g g`")),
            "a shadowed binding never reached the reported problems: {problems:?}"
        );
    }

    #[test]
    fn typing_never_triggers_a_single_key_action() {
        let mut resolver = resolver();
        let now = Instant::now();

        for character in "already replied".chars().filter(|c| !c.is_whitespace()) {
            let outcome = resolver.press(
                &Chord::new(Key::Char(character), Modifiers::NONE),
                KeyContext::List,
                true,
                now,
            );
            assert_eq!(
                outcome,
                Outcome::Unhandled,
                "`{character}` fired while the user was typing"
            );
        }
        assert_eq!(resolver.pending(), None, "and started no sequence either");
    }

    #[test]
    fn typing_does_not_start_a_sequence() {
        let mut resolver = resolver();
        let now = Instant::now();

        assert_eq!(
            resolver.press(&chord("g"), KeyContext::List, true, now),
            Outcome::Unhandled
        );
        assert_eq!(resolver.pending(), None);
    }

    #[test]
    fn modified_bindings_still_fire_while_typing() {
        let mut resolver = resolver();
        let now = Instant::now();

        assert_eq!(
            resolver.press(&chord("ctrl+k"), KeyContext::Composer, true, now),
            Outcome::Command("command_palette".to_owned()),
            "the palette has to be reachable from inside the composer"
        );
        assert_eq!(
            resolver.press(&chord("ctrl+Return"), KeyContext::Composer, true, now),
            Outcome::Command("send".to_owned())
        );
    }

    #[test]
    fn the_composer_keeps_its_own_keys_while_the_forward_chooser_holds_the_keyboard() {
        let mut resolver = resolver();
        let now = Instant::now();

        for (key, command) in [("Escape", "close_composer"), ("ctrl+Return", "send")] {
            assert_eq!(
                resolver.press(&chord(key), KeyContext::ForwardChooser, false, now),
                Outcome::Command(command.to_owned()),
                "{key} still reaches the composer from the chooser"
            );
        }
    }

    #[test]
    fn escape_still_fires_while_typing() {
        let mut resolver = resolver();
        let now = Instant::now();

        assert_eq!(
            resolver.press(&chord("Escape"), KeyContext::Composer, true, now),
            Outcome::Command("close_composer".to_owned()),
            "Escape is how the composer is closed, and it is always typing"
        );
    }

    #[test]
    fn return_and_tab_belong_to_the_text_field() {
        let mut resolver = resolver();
        let now = Instant::now();

        assert_eq!(
            resolver.press(&chord("Return"), KeyContext::List, true, now),
            Outcome::Unhandled,
            "Return inserts a newline; ctrl+Return is what sends"
        );
        assert!(!chord("Tab").survives_text_entry());
        assert!(!chord("Up").survives_text_entry(), "the caret moves");
        assert!(chord("F5").survives_text_entry());
        assert!(!chord("shift+Tab").survives_text_entry());
    }

    #[test]
    fn rebinding_replaces_rather_than_stacks() {
        let mut keymap = canvas_keymap();
        keymap
            .bind(KeyContext::List, "a", "something_else")
            .unwrap();

        let matches: Vec<&str> = keymap
            .entries()
            .filter(|(context, binding, _)| {
                *context == KeyContext::List && binding.to_string() == "a"
            })
            .map(|(_, _, command)| command)
            .collect();

        assert_eq!(matches, vec!["something_else"]);
    }

    #[test]
    fn a_binding_can_be_looked_up_by_its_command_for_the_cheat_sheet() {
        let keymap = canvas_keymap();

        assert_eq!(
            keymap
                .binding_for(KeyContext::List, "archive_thread")
                .map(Binding::to_string),
            Some("A".to_owned())
        );
        assert_eq!(keymap.binding_for(KeyContext::List, "teleport"), None);
    }

    #[test]
    fn replacing_the_table_drops_a_half_typed_sequence() {
        let mut resolver = resolver();
        let now = Instant::now();
        resolver.press(&chord("g"), KeyContext::List, false, now);

        resolver.set_keymap(canvas_keymap());

        assert_eq!(resolver.pending(), None);
    }
}

#[cfg(test)]
mod unexpanded_mod_tests {
    use super::*;

    #[test]
    fn the_token_is_rejected_with_a_message_that_names_the_fix() {
        // A literal `mod+k` match would work where nothing expanded it and silently differ from
        // other surfaces.
        let error = "mod+k".parse::<Binding>().expect_err("mod must not parse");
        assert!(matches!(error, ParseError::UnexpandedMod));
        assert!(
            error.to_string().contains("expand_mod"),
            "the message has to name the fix: {error}"
        );
    }

    #[test]
    fn ctrl_still_parses() {
        assert!("ctrl+k".parse::<Binding>().is_ok());
    }
}

#[cfg(test)]
mod keysym_tests {
    use super::Key;

    #[test]
    fn punctuation_letters_and_named_keys_spell_as_keysyms() {
        assert_eq!(Key::Char('?').keysym_name(), "question");
        assert_eq!(Key::Char('+').keysym_name(), "plus");
        assert_eq!(Key::Char('a').keysym_name(), "a");
        assert_eq!(Key::Char('7').keysym_name(), "7");
        assert_eq!(Key::Named("Space").keysym_name(), "space");
        assert_eq!(Key::Named("Return").keysym_name(), "Return");
    }
}
