//! Configuration, as the runtime consumes it.
//!
//! `config.toml` *is* the settings — there is no second store, the settings
//! panel edits the same file, and the design promises *applied live · nothing
//! to save*. Core owns the live handle, resolves `[keys]` onto the command
//! registry to build the [`Keymap`] every surface reads, and re-emits what
//! changed as events.
//!
//! # A broken file is not a broken application
//!
//! `postio-config` validates off the watcher thread and keeps the last good
//! configuration in force when the file will not load. Core inherits that: a
//! rejected reload leaves the keymap exactly as it was and reports the problem
//! as an [`Event::Error`], so the user still has working keys to fix the file
//! with — including `Ctrl+E`.
//!
//! # Only what changed
//!
//! Reapplying everything on every keystroke would be visibly slow, so a reload
//! reports a [`ConfigChange`] naming the sections that actually moved, and a
//! save that changes nothing anybody is waiting on emits nothing at all.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use postio_config::change::ConfigChanged;
use postio_config::live::{LiveConfig, Reload};
use postio_config::paths::Platform;
use postio_config::validate::{Checked, Validation};
use postio_config::watch::{ConfigWatcher, WatchOptions};
use postio_config::{Config, ConfigError, KeyBindings, keys};

use crate::bridge::EventSink;
use crate::{ActionId, Context, ContextSet, Event, registry};

/// Which sections of the configuration moved.
///
/// The diff itself lives in `postio_config::change` — pure comparison logic
/// belongs with the schema it compares, not with the runtime that reacts to
/// it. This alias keeps the name `postio_core` consumers already use.
pub type ConfigChange = ConfigChanged;

/// The bindings in force: the registry's defaults with `[keys]` applied.
///
/// The keymap resolver, the `Ctrl+K` palette, the `?` cheat sheet and the key
/// hints on the focused row all read this, so a rebind reaches every surface at
/// once. Bindings stay untyped strings here — parsing `"ctrl+k"` into a GDK
/// accelerator is `postio-gtk`'s job, and core must not learn about GDK.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<ActionId, Vec<String>>,
    problems: Vec<String>,
}

impl Keymap {
    /// Resolve `[keys]` onto the command registry.
    ///
    /// An override that cannot be used — an unusable binding, a command this
    /// build does not know, a key already taken in the same context — is a
    /// warning and never a failure: the command keeps its default binding, or
    /// loses its key and stays reachable from the palette.
    pub fn resolve(overrides: &KeyBindings) -> Self {
        Self::resolve_on(overrides, Platform::host())
    }

    /// [`resolve`](Self::resolve) for a named platform.
    ///
    /// `mod` is expanded here, once, so everything downstream — the resolver,
    /// the conflict check, the cheat sheet, the key hints — sees a concrete
    /// accelerator and no layer below needs to know the word. Conflict
    /// detection in particular depends on it: `mod+k` and `ctrl+k` are the
    /// same key on Linux, and comparing the unexpanded strings would let both
    /// be claimed.
    ///
    /// Taking the platform as a parameter rather than reading a `cfg` is what
    /// lets either host assert both answers, the same discipline the path
    /// resolution uses.
    pub fn resolve_on(overrides: &KeyBindings, platform: Platform) -> Self {
        let mut keymap = Keymap::default();
        for spec in registry::every_action() {
            keymap.bindings.insert(spec.id, Vec::new());
        }
        // Every id `[keys]` names that parses, whether or not a command exists
        // for it yet. This is what makes a binding written before its
        // extension loads survive: the id is bound now and starts reaching a
        // command the moment one registers. See the `ActionId` module note.
        for command in overrides.overrides().keys() {
            if let Ok(action) = command.parse::<ActionId>() {
                keymap.bindings.entry(action).or_default();
            }
        }

        // Two passes, because an explicit `[keys]` entry outranks a built-in
        // default. One pass would give a contested key to whichever command
        // the registry happens to list first, so adding a command with a
        // popular default could quietly take a key the user had already asked
        // for — and which of the two won would depend on the order of a table
        // they have never seen. A default is a suggestion; an override is not.
        //
        // Within each pass, registry order decides, so the result is
        // deterministic rather than dependent on map iteration order.
        for spec in keymap.known_actions() {
            let Some(binding) = overrides.overrides().get(spec.as_str()).cloned() else {
                continue;
            };
            let binding = keys::expand_mod(&binding, platform);
            let binding = binding.as_str();
            let contexts = keymap.contexts_of(spec);
            if let Some(problem) = keys::binding_problem(binding) {
                keymap.problems.push(format!(
                    "`{binding}` is not usable as the binding for `{spec}`: {problem}"
                ));
                continue;
            }
            let binding = binding.trim().to_string();
            if let Some(taken) = keymap.holder_of(&binding, contexts) {
                keymap.problems.push(format!(
                    "`{binding}` is already bound to `{taken}`, so `{spec}` keeps its default"
                ));
                continue;
            }
            keymap.claim(spec, binding);
        }

        for spec in registry::every_action() {
            if keymap.binding(spec.id).is_some() {
                continue;
            }
            let Some(default) = spec
                .default_binding
                .map(|binding| keys::expand_mod(binding, platform))
            else {
                continue;
            };
            if let Some(taken) = keymap.holder_of(&default, spec.contexts) {
                // Palette-only rather than shadowing someone else's key. Said
                // out loud, because a command that quietly lost its key is a
                // command the user will press and be ignored by.
                keymap.problems.push(format!(
                    "`{default}` is already bound to `{taken}`, so `{}` has no key \
                     and stays reachable from the palette",
                    spec.id
                ));
                continue;
            }
            keymap.claim(spec.id, default);
        }

        // Alternates last, and only where nothing else wanted them: a second
        // way to reach a command must never cost another command its first.
        for spec in registry::every_action() {
            for alternate in spec.alternate_bindings {
                let alternate = keys::expand_mod(alternate, platform);
                if keymap.holder_of(&alternate, spec.contexts).is_none() {
                    keymap.claim(spec.id, alternate);
                }
            }
        }

        for command in overrides.overrides().keys() {
            // An id that does not parse at all is worth reporting. An
            // extension id that parses but has not registered yet is not: it
            // is the expected state at startup, and warning about it would
            // train the user to ignore the validity line.
            if command.parse::<ActionId>().is_err() {
                // Not fatal on purpose: a binding written by a newer Postio
                // survives a downgrade and an upgrade round trip.
                keymap
                    .problems
                    .push(format!("`{command}` is not a command in this build"));
            }
        }

        keymap
    }

    /// Every action the keymap holds a row for, in resolution order.
    ///
    /// Registered commands first in registry order, then any id `[keys]`
    /// named that nothing has registered — the latter cannot be ordered by a
    /// registry that has never seen them.
    fn known_actions(&self) -> Vec<ActionId> {
        self.bindings.keys().copied().collect()
    }

    /// The contexts an action applies in.
    ///
    /// [`ContextSet::ANY`] for an extension id nothing has registered yet.
    /// Conservative on purpose and in the user's favour: an explicit `[keys]`
    /// entry outranks a built-in default (see the two-pass note above), so an
    /// id whose contexts are not yet known must be assumed to overlap rather
    /// than assumed harmless — otherwise a built-in would quietly claim the
    /// same key and the user's binding would be the one that lost.
    fn contexts_of(&self, action: ActionId) -> ContextSet {
        registry::spec(action)
            .map(|spec| spec.contexts)
            .unwrap_or(ContextSet::ANY)
    }

    /// The primary binding for a command, if it has one.
    ///
    /// Takes anything that names an action, so the many call sites that pass a
    /// built-in `CommandId` read exactly as they did before the vocabulary
    /// widened.
    pub fn binding(&self, command: impl Into<ActionId>) -> Option<&str> {
        self.bindings
            .get(&command.into())
            .and_then(|bindings| bindings.first())
            .map(String::as_str)
    }

    /// Every binding for a command, the primary first.
    pub fn bindings(&self, command: impl Into<ActionId>) -> &[String] {
        self.bindings
            .get(&command.into())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The command a key reaches in a context, if any.
    pub fn command_for(&self, context: Context, binding: &str) -> Option<ActionId> {
        registry::reachable(context)
            .find(|spec| {
                self.bindings(spec.id)
                    .iter()
                    .any(|candidate| candidate == binding)
            })
            .map(|spec| spec.id)
    }

    /// What could not be honoured, phrased for the settings validity line.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Give `command` a binding, after the caller has checked it is free.
    fn claim(&mut self, command: ActionId, binding: String) {
        self.bindings.entry(command).or_default().push(binding);
    }

    fn holder_of(&self, binding: &str, contexts: ContextSet) -> Option<ActionId> {
        self.bindings.iter().find_map(|(command, bindings)| {
            let overlaps = self.contexts_of(*command).intersects(contexts);
            let matches = bindings.iter().any(|candidate| candidate == binding);
            (overlaps && matches).then_some(*command)
        })
    }
}

/// What a reload did, and what the UI should be told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigUpdate {
    /// Whether the file was applied, identical or rejected.
    pub reload: Reload,
    /// The sections that moved. Empty when nothing was applied.
    pub changed: ConfigChange,
    /// The events to emit — a reload naming the change, or an error.
    pub events: Vec<Event>,
}

impl ConfigUpdate {
    /// Whether a new configuration took effect.
    pub fn applied(&self) -> bool {
        self.reload == Reload::Applied
    }
}

/// The configuration handle core owns: the file in force, plus the keymap
/// derived from it.
#[derive(Debug, Clone)]
pub struct ConfigService {
    live: LiveConfig,
    keymap: Keymap,
}

impl ConfigService {
    /// Start from defaults without touching the disk.
    pub fn new(path: &Path) -> Self {
        let live = LiveConfig::new(path);
        let keymap = Keymap::resolve(&live.config().keys);
        ConfigService { live, keymap }
    }

    /// Load `path`. A missing or broken file yields working defaults — first
    /// run needs nothing on disk, and a broken file still leaves an app to fix
    /// it in.
    pub fn load(path: &Path) -> Self {
        let live = LiveConfig::load(path);
        let keymap = Keymap::resolve(&live.config().keys);
        ConfigService { live, keymap }
    }

    /// The configuration in force.
    pub fn config(&self) -> &Config {
        self.live.config()
    }

    /// The bindings in force.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// The status of the file as last read — what the validity line shows.
    pub fn status(&self) -> &Validation {
        self.live.status()
    }

    /// The file being watched.
    pub fn path(&self) -> &Path {
        self.live.path()
    }

    /// Re-read the file from disk. Prefer [`apply`](Self::apply) with a
    /// [`Checked`] from the watcher: parsing belongs off the UI thread.
    pub fn reload(&mut self) -> ConfigUpdate {
        let checked = postio_config::validate::check_path(self.live.path());
        self.apply(checked)
    }

    /// Adopt a configuration the watcher has already parsed and validated.
    pub fn apply(&mut self, checked: Checked) -> ConfigUpdate {
        let before = self.live.config().clone();
        let reload = self.live.apply(checked);

        let mut update = ConfigUpdate {
            reload,
            changed: ConfigChange::default(),
            events: Vec::new(),
        };

        match reload {
            Reload::Rejected => {
                // The last good configuration — and so the last good keymap —
                // stays exactly as it is.
                let message = self
                    .live
                    .status()
                    .first_error()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "the configuration could not be loaded".to_string());
                update.events.push(Event::Error { message });
            }
            Reload::Unchanged => {}
            Reload::Applied => {
                update.changed = ConfigChange::between(&before, self.live.config());
                if update.changed.keys {
                    self.keymap = Keymap::resolve(&self.live.config().keys);
                    for problem in self.keymap.problems() {
                        update.events.push(Event::Error {
                            message: problem.clone(),
                        });
                    }
                }
                // A key no subsystem is waiting on — one written by a newer
                // Postio, say — is worth preserving but not repainting for.
                if update.changed.any() {
                    update.events.push(Event::ConfigReloaded {
                        changed: update.changed,
                    });
                }
            }
        }

        update
    }
}

/// The configuration as the runtime shares it: one owner, many readers.
///
/// The watcher parses on its own thread and applies through this handle, so the
/// only work the UI thread does is repaint from the events that come back.
#[derive(Debug, Clone)]
pub struct SharedConfig(Arc<Mutex<ConfigService>>);

impl SharedConfig {
    /// Share a handle.
    pub fn new(service: ConfigService) -> Self {
        SharedConfig(Arc::new(Mutex::new(service)))
    }

    /// Load `path` and share the result.
    pub fn load(path: &Path) -> Self {
        SharedConfig::new(ConfigService::load(path))
    }

    /// Read something out of the configuration in force.
    pub fn read<R>(&self, with: impl FnOnce(&ConfigService) -> R) -> R {
        with(&self.lock())
    }

    /// A copy of the bindings, for a consumer that must not hold the lock.
    pub fn keymap(&self) -> Keymap {
        self.lock().keymap.clone()
    }

    /// Apply a parsed configuration and emit what the UI needs to know.
    pub fn apply(&self, checked: Checked, events: &EventSink) -> ConfigUpdate {
        let update = self.lock().apply(checked);
        for event in &update.events {
            events.emit(event.clone());
        }
        update
    }

    /// Watch the file and apply saves as they land.
    ///
    /// The watch covers the containing directory, so it survives the
    /// write-to-temp-and-rename dance every careful editor does; parsing and
    /// validating happen on the watcher's thread. Dropping the returned
    /// watcher stops it.
    pub fn watch(&self, events: EventSink) -> Result<ConfigWatcher, ConfigError> {
        self.watch_with(WatchOptions::default(), events)
    }

    /// Watch the file, choosing the debounce.
    pub fn watch_with(
        &self,
        options: WatchOptions,
        events: EventSink,
    ) -> Result<ConfigWatcher, ConfigError> {
        let path = self.lock().path().to_path_buf();
        let config = self.clone();
        ConfigWatcher::with_options(&path, options, move |checked| {
            config.apply(checked, &events);
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfigService> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keymap_covers_every_command_in_the_registry() {
        let keymap = Keymap::resolve(&KeyBindings::default());
        for spec in registry::all() {
            assert_eq!(
                keymap.binding(spec.id),
                Some(keys::expand_mod(spec.default_binding, Platform::host()).as_str()),
                "`{}` did not keep its default",
                spec.id
            );
        }
    }

    #[test]
    fn a_change_is_named_section_by_section() {
        let old = Config::from_toml_str("[ui]\ndensity = \"compact\"\n").expect("valid");
        let new = Config::from_toml_str("[ui]\ndensity = \"comfortable\"\n").expect("valid");

        let changed = ConfigChange::between(&old, &new);
        assert!(changed.ui);
        assert!(changed.any());
        assert!(!changed.keys);
        assert_eq!(ConfigChange::between(&old, &old), ConfigChange::default());
    }
}
