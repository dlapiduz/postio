//! The keymap resolver: single keys, chords, sequences and contexts.
//!
//! # Why not GTK accelerators
//!
//! Postio's bindings come from the design canvas, and three of its rules are
//! outside what `gtk::ShortcutController` can express:
//!
//! * **`gg` versus `G`.** A sequence of two presses is not an accelerator, and
//!   while `g` is pending the user has to be *shown* that it is pending.
//! * **`Esc` means different things.** In the list it clears the selection, in
//!   a thread it goes back up, in the composer it closes the composer. One key,
//!   several bindings, resolved by [`KeyContext`].
//! * **Typing must always win.** A single-key binding like `a` cannot fire
//!   while the user is typing a subject line, and no accelerator arrangement
//!   makes that reliable — the resolver has to be asked.
//!
//! So this module owns the decision and the widgets only report key presses to
//! it. Nothing here touches a widget, which is what lets it be unit-tested with
//! no display and no GTK main loop.
//!
//! # Shape
//!
//! [`Keymap`] is the table: `(context, binding, command)`. [`Resolver`] is the
//! live state on top of it — the pending chords of a half-typed sequence and
//! when they expire. A press goes in, an [`Outcome`] comes out.
//!
//! ```
//! use postio_gtk::keymap::{KeyContext, Keymap, Outcome, Resolver};
//! use std::time::Instant;
//!
//! let mut keymap = Keymap::new();
//! keymap.bind(KeyContext::List, "g g", "go_to_top").unwrap();
//! keymap.bind(KeyContext::List, "G", "go_to_bottom").unwrap();
//! let mut resolver = Resolver::new(keymap);
//!
//! let now = Instant::now();
//! assert!(matches!(
//!     resolver.press(&"g".parse().unwrap(), KeyContext::List, false, now),
//!     Outcome::Pending(_)
//! ));
//! assert_eq!(
//!     resolver.press(&"g".parse().unwrap(), KeyContext::List, false, now),
//!     Outcome::Command("go_to_top".into())
//! );
//! ```

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use gtk::gdk;
use postio_core::{Context, ContextSet, registry};

/// How long a half-typed sequence stays pending before it is forgotten.
///
/// Long enough to be typed deliberately, short enough that a `g` left over
/// from a moment ago never swallows the next keystroke. The pending state is
/// visible while it lasts, so the user is never guessing.
pub const CHORD_TIMEOUT: Duration = Duration::from_millis(1_000);

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

/// The modifier keys held down with a chord.
///
/// `Shift` is deliberately *not* one of them for printable keys: the canvas
/// distinguishes `a` from `A`, and the character itself already says which.
/// See [`Chord`] for how that is normalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Control.
    pub const CTRL: Self = Self(1 << 0);
    /// Alt.
    pub const ALT: Self = Self(1 << 1);
    /// Shift. Only ever set on a named key; see [`Chord`].
    pub const SHIFT: Self = Self(1 << 2);
    /// Super / Meta / the Windows key.
    pub const SUPER: Self = Self(1 << 3);

    /// Whether every modifier in `other` is set here.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether nothing is held.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// This set with `other` added.
    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set with `other` removed.
    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::CTRL),
            "alt" => Some(Self::ALT),
            "shift" => Some(Self::SHIFT),
            "super" | "meta" => Some(Self::SUPER),
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

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Named keys that do not produce a character.
///
/// The left column is every spelling accepted in a binding string; the right is
/// the one spelling everything is stored and displayed as, so `esc` and
/// `Escape` are the same key rather than two that never match each other.
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

/// Keys that are spelled by name but *are* a character.
///
/// `?` and `question` have to be the same chord, or a binding copied out of a
/// GDK key table would never match a key the user can actually press.
const PUNCTUATION_NAMES: &[(&str, char)] = &[
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
    ("underscore", '_'),
    ("less", '<'),
    ("greater", '>'),
    ("bracketleft", '['),
    ("bracketright", ']'),
    ("grave", '`'),
    ("apostrophe", '\''),
    ("quotedbl", '"'),
];

/// One key on the keyboard.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A key that produces a character. Case matters: `a` and `A` differ.
    Char(char),
    /// A key that does not, stored as its canonical GDK name.
    Named(&'static str),
}

impl Key {
    /// Whether this key still fires a binding while a text field has focus.
    ///
    /// Only `Escape` and the function keys do. Everything else a text field
    /// might reasonably consume — characters, `Return`, `Tab`, the arrows,
    /// `Home`/`End` — belongs to the text field while it is focused.
    fn survives_text_entry(&self) -> bool {
        match self {
            Self::Char(_) => false,
            Self::Named(name) => {
                *name == "Escape" || (name.starts_with('F') && name[1..].parse::<u8>().is_ok())
            }
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

// ---------------------------------------------------------------------------
// Chords and bindings
// ---------------------------------------------------------------------------

/// One press: a key with the modifiers held down for it.
///
/// # Normalization
///
/// A chord is always stored in the form the keyboard will actually deliver it.
/// For a key that produces a character, `Shift` is folded into the character —
/// `shift+a` *is* `A` — because that is what the toolkit reports and because
/// the canvas binds `a` and `A` to different commands. For a named key, `Shift`
/// stays a modifier, since `shift+Tab` produces no character to fold it into.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    /// The key pressed.
    pub key: Key,
    /// What was held down with it.
    pub modifiers: Modifiers,
}

impl Chord {
    /// Builds a chord, normalizing `Shift` as described on the type.
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        match key {
            Key::Char(character) if modifiers.contains(Modifiers::SHIFT) => {
                // `shift+p` and `P` have to be one chord, not two that never
                // match each other.
                let upper = character.to_uppercase().next().unwrap_or(character);
                Self {
                    key: Key::Char(upper),
                    modifiers: modifiers.without(Modifiers::SHIFT),
                }
            }
            key => Self { key, modifiers },
        }
    }

    /// Builds a chord from a GTK key-pressed event.
    ///
    /// This is the whole of the bridge between the toolkit and the resolver:
    /// a widget's key controller reports `(keyval, state)` and everything from
    /// here on is testable without one. Returns `None` for a key this build has
    /// no name for — a dead key, a keypad key with no binding — which the
    /// caller should let propagate.
    pub fn from_key_event(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Self> {
        let mut modifiers = Modifiers::NONE;
        for (mask, modifier) in [
            (gdk::ModifierType::CONTROL_MASK, Modifiers::CTRL),
            (gdk::ModifierType::ALT_MASK, Modifiers::ALT),
            (gdk::ModifierType::SHIFT_MASK, Modifiers::SHIFT),
            (gdk::ModifierType::SUPER_MASK, Modifiers::SUPER),
        ] {
            if state.contains(mask) {
                modifiers = modifiers.with(modifier);
            }
        }

        if let Some(character) = keyval.to_unicode() {
            // Space has a character but no useful one to print in a cheat
            // sheet, so it stays a named key.
            if character == ' ' {
                return Some(Self::new(Key::Named("Space"), modifiers));
            }
            if !character.is_control() {
                return Some(Self::new(Key::Char(character), modifiers));
            }
        }
        Key::parse(keyval.name()?.as_str()).map(|key| Self::new(key, modifiers))
    }

    /// Whether this chord fires while a text field has focus.
    ///
    /// True when it carries `Ctrl`, `Alt` or `Super`, or when its key is one
    /// the text field would not want anyway. This is the rule that keeps `a`
    /// from archiving mail while the user types "already replied".
    pub fn survives_text_entry(&self) -> bool {
        // Shift alone does not make a chord a shortcut: `shift+a` is still the
        // letter A to a text field.
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

/// A whole binding: one chord, or a sequence of them like `g g`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Binding(Vec<Chord>);

impl Binding {
    /// The chords, in the order they must be pressed.
    pub fn chords(&self) -> &[Chord] {
        &self.0
    }

    /// The first chord, which is what the text-entry rule is applied to.
    pub fn first(&self) -> &Chord {
        &self.0[0]
    }

    /// How many presses it takes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Never true: a binding always has at least one chord.
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

/// Why a binding string could not be read.
///
/// The messages are written to be shown to the user beside the `[keys]` entry
/// they came from — a binding this build cannot parse is a settings problem,
/// not a crash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The binding string was empty or all whitespace.
    #[error("a binding cannot be empty")]
    Empty,
    /// A `+`-separated part was not a modifier.
    #[error("`{modifier}` is not a modifier; use ctrl, alt, shift or super")]
    UnknownModifier {
        /// What was written.
        modifier: String,
    },
    /// The chord ended with a modifier and no key.
    #[error("`{chord}` ends with a modifier and no key")]
    NoKey {
        /// The chord as written.
        chord: String,
    },
    /// The key name is not one this build knows.
    #[error("`{key}` is not a key name")]
    UnknownKey {
        /// What was written.
        key: String,
    },
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

/// Where the keyboard focus is, which decides what a key means.
///
/// Contexts fall back rather than replace: reading a message still leaves the
/// list's `j`/`k` working, because that is what the canvas draws. The chain is
/// [`KeyContext::chain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum KeyContext {
    /// Bindings that apply everywhere unless something more specific claims
    /// the key.
    #[default]
    Global,
    /// The message list.
    List,
    /// A thread drilled into from the list.
    Thread,
    /// The reading pane.
    Reader,
    /// The composer, which takes over the reading pane.
    Composer,
    /// The search bar.
    Search,
    /// The command palette.
    Palette,
}

impl KeyContext {
    /// The contexts to look in, most specific first.
    ///
    /// The precedence order *is* this chain. Composer, Search and Palette do
    /// not fall back through the list: while the composer is open, `a` is the
    /// letter A, not archive — and that is enforced here as well as by the
    /// text-entry rule, so a non-text widget inside the composer cannot
    /// accidentally reopen the hole.
    pub fn chain(self) -> &'static [KeyContext] {
        match self {
            Self::Global => &[Self::Global],
            Self::List => &[Self::List, Self::Global],
            Self::Thread => &[Self::Thread, Self::List, Self::Global],
            Self::Reader => &[Self::Reader, Self::Thread, Self::List, Self::Global],
            Self::Composer => &[Self::Composer, Self::Global],
            Self::Search => &[Self::Search, Self::Global],
            Self::Palette => &[Self::Palette, Self::Global],
        }
    }
}

impl From<Context> for KeyContext {
    /// The runtime's context, as the resolver names it.
    ///
    /// One-to-one. [`KeyContext::Global`] has no counterpart on purpose: core
    /// spells "everywhere" as `ContextSet::ANY` over the six real contexts,
    /// and collapsing that to a single fallback entry is
    /// [`Keymap::from_commands`]'s job, not a variant core has to carry.
    fn from(context: Context) -> Self {
        match context {
            Context::List => Self::List,
            Context::Thread => Self::Thread,
            Context::Reader => Self::Reader,
            Context::Composer => Self::Composer,
            Context::Search => Self::Search,
            Context::Palette => Self::Palette,
        }
    }
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// The binding table: which command a binding runs, in which context.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    entries: Vec<(KeyContext, Binding, String)>,
}

impl Keymap {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds a command in a context, replacing any binding already there.
    ///
    /// Rebinding the same `(context, binding)` replaces it rather than
    /// stacking, so the user's `[keys]` override wins over the default without
    /// the caller having to remove anything first.
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

    /// Removes a binding, returning whether there was one.
    pub fn unbind(&mut self, context: KeyContext, binding: &Binding) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|(existing, bound, _)| !(*existing == context && bound == binding));
        self.entries.len() != before
    }

    /// The binding a command has in a context, if any.
    pub fn binding_for(&self, context: KeyContext, command: &str) -> Option<&Binding> {
        self.entries
            .iter()
            .find(|(existing, _, bound)| *existing == context && bound == command)
            .map(|(_, binding, _)| binding)
    }

    /// Every entry, for the cheat sheet and the command palette.
    pub fn entries(&self) -> impl Iterator<Item = (KeyContext, &Binding, &str)> {
        self.entries
            .iter()
            .map(|(context, binding, command)| (*context, binding, command.as_str()))
    }

    /// Bindings that can never fire because a shorter one in the same context
    /// always matches first — `g` shadowing `g g`.
    ///
    /// Not an error at bind time: a user override can create one, and refusing
    /// to start over it would be worse than reporting it. The settings panel
    /// shows these.
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

    /// Builds the whole binding table from the command registry.
    ///
    /// This is where the canvas's default binding set becomes something the
    /// resolver can act on. `commands` is [`postio_core::Keymap`], which has
    /// already resolved `[keys]` overrides onto the registry and dropped
    /// collisions — so the defaults live in the registry as data, exactly once,
    /// and the user's file wins before this is ever called.
    ///
    /// A command reachable in every context is bound once in
    /// [`KeyContext::Global`] rather than six times, so a fallback context can
    /// still claim the key for something more specific.
    ///
    /// Returns the table and whatever could not be parsed. A binding this build
    /// cannot read costs its command a key and is reported; it never stops the
    /// application, which would leave the user with no way to fix it.
    pub fn from_commands(commands: &postio_core::Keymap) -> (Self, Vec<String>) {
        let mut keymap = Self::new();
        let mut problems = Vec::new();

        for spec in registry::all() {
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
                    // An exact match wins immediately, even when a longer
                    // binding also starts this way: waiting for a timeout
                    // before acting on a key the user has fully typed reads as
                    // lag, and `shadowed` reports the arrangement instead.
                    return Match::Exact(command.as_str());
                }
                if binding.starts_with(pressed) {
                    prefix = true;
                }
            }
            if prefix {
                // A more specific context has claimed this prefix; do not let a
                // fallback context resolve it out from under the sequence.
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

// ---------------------------------------------------------------------------
// The resolver
// ---------------------------------------------------------------------------

/// What a key press meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Run this command.
    Command(String),
    /// The start of a sequence. The string is what to show the user while it
    /// is pending — `"g"` — so the pending state is never invisible.
    Pending(String),
    /// Nothing is bound to it. The caller should let the key propagate.
    Unhandled,
}

/// The live keymap: the table plus whatever sequence is half-typed.
#[derive(Debug)]
pub struct Resolver {
    keymap: Keymap,
    timeout: Duration,
    pending: Vec<Chord>,
    /// When the pending sequence stops being pending.
    expires_at: Option<Instant>,
}

impl Resolver {
    /// Wraps a keymap, with the default [`CHORD_TIMEOUT`].
    pub fn new(keymap: Keymap) -> Self {
        Self {
            keymap,
            timeout: CHORD_TIMEOUT,
            pending: Vec::new(),
            expires_at: None,
        }
    }

    /// A resolver over the registry defaults with `[keys]` already applied.
    ///
    /// The companion to [`Keymap::from_commands`], returning whatever could
    /// not be honoured so the caller can put it on the settings validity line.
    pub fn from_commands(commands: &postio_core::Keymap) -> (Self, Vec<String>) {
        let (keymap, problems) = Keymap::from_commands(commands);
        (Self::new(keymap), Self::all_problems(commands, problems))
    }

    /// Rebuilds the table after `config.toml` changed, without a restart.
    ///
    /// Called when a reload reports `ConfigChange { keys: true }`. Returns the
    /// problems to report — core's first, since "`y` is already bound to
    /// `flag`" is what the user needs to hear, and this crate's parse failures
    /// after.
    ///
    /// Everything downstream — the palette, the cheat sheet, the key hints —
    /// reads [`postio_core::Keymap`] directly and so follows on its own; this
    /// is only the half that has to be reparsed into chords.
    pub fn apply_commands(&mut self, commands: &postio_core::Keymap) -> Vec<String> {
        let (keymap, problems) = Keymap::from_commands(commands);
        self.set_keymap(keymap);
        Self::all_problems(commands, problems)
    }

    fn all_problems(
        commands: &postio_core::Keymap,
        mut parse_problems: Vec<String>,
    ) -> Vec<String> {
        let mut problems = commands.problems().to_vec();
        problems.append(&mut parse_problems);
        problems
    }

    /// Wraps a keymap with a different pending-sequence timeout.
    pub fn with_timeout(keymap: Keymap, timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::new(keymap)
        }
    }

    /// The table this resolver reads.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Replaces the table, e.g. after the config file changed on disk.
    ///
    /// Any half-typed sequence is dropped: it was typed against the old table.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
        self.clear_pending();
    }

    /// What is currently half-typed, for the pending indicator.
    pub fn pending(&self) -> Option<String> {
        (!self.pending.is_empty()).then(|| Self::describe(&self.pending))
    }

    /// Forgets a half-typed sequence — on focus loss, or when the user hits
    /// `Escape` and the caller decides that means "never mind".
    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.expires_at = None;
    }

    /// Resolves one key press.
    ///
    /// `in_text_entry` is whether the focused widget takes text. When it is
    /// true, a binding whose first chord is a bare character or an ordinary
    /// editing key does not fire at all — typing always wins.
    ///
    /// `now` is passed in rather than read so the timeout is testable without
    /// sleeping.
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
                // A sequence that went nowhere is abandoned whole, rather than
                // re-tried as a fresh first chord: `g` then `q` must not
                // quietly run whatever `q` is bound to.
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
        // The canvas bindings, per context.
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
        keymap.bind(KeyContext::Thread, "Escape", "back").unwrap();
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

    // -----------------------------------------------------------------------
    // Parsing
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Single keys and contexts
    // -----------------------------------------------------------------------

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
            (KeyContext::Thread, "back"),
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

    // -----------------------------------------------------------------------
    // Sequences
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Text entry
    // -----------------------------------------------------------------------

    #[test]
    fn typing_never_triggers_a_single_key_action() {
        let mut resolver = resolver();
        let now = Instant::now();

        // "already replied" — every letter of it, in the list's own context.
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

    // -----------------------------------------------------------------------
    // The GTK bridge
    // -----------------------------------------------------------------------

    #[test]
    fn a_key_event_becomes_the_chord_the_binding_was_written_as() {
        for (name, state, expected) in [
            ("a", gdk::ModifierType::empty(), "a"),
            ("A", gdk::ModifierType::SHIFT_MASK, "A"),
            ("k", gdk::ModifierType::CONTROL_MASK, "ctrl+k"),
            ("Return", gdk::ModifierType::empty(), "Return"),
            ("Return", gdk::ModifierType::CONTROL_MASK, "ctrl+Return"),
            ("Escape", gdk::ModifierType::empty(), "Escape"),
            ("Tab", gdk::ModifierType::SHIFT_MASK, "shift+Tab"),
            ("question", gdk::ModifierType::SHIFT_MASK, "?"),
            ("slash", gdk::ModifierType::empty(), "/"),
            ("Page_Up", gdk::ModifierType::empty(), "Page_Up"),
            ("F5", gdk::ModifierType::empty(), "F5"),
            ("space", gdk::ModifierType::empty(), "Space"),
        ] {
            let built = Chord::from_key_event(gdk::Key::from_name(name).unwrap(), state)
                .unwrap_or_else(|| panic!("{name} produced no chord"));
            assert_eq!(built, chord(expected), "{name} with {state:?}");
        }
    }

    #[test]
    fn a_key_event_resolves_the_command_the_canvas_binds_to_it() {
        let mut resolver = resolver();
        let now = Instant::now();

        let pressed = Chord::from_key_event(
            gdk::Key::from_name("A").unwrap(),
            gdk::ModifierType::SHIFT_MASK,
        )
        .expect("a chord");

        assert_eq!(
            resolver.press(&pressed, KeyContext::List, false, now),
            Outcome::Command("archive_thread".to_owned()),
            "shift+a is the canvas's A"
        );
    }

    // -----------------------------------------------------------------------
    // The table
    // -----------------------------------------------------------------------

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
    fn unbinding_removes_the_entry() {
        let mut keymap = canvas_keymap();
        let binding: Binding = "a".parse().unwrap();

        assert!(keymap.unbind(KeyContext::List, &binding));
        assert!(!keymap.unbind(KeyContext::List, &binding), "already gone");

        let mut resolver = Resolver::new(keymap);
        assert_eq!(
            press(&mut resolver, "a", KeyContext::List),
            Outcome::Unhandled
        );
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
