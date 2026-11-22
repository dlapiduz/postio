//! Configuration as the runtime consumes it: `config.toml` is the settings, and core resolves
//! `[keys]` into the `Keymap`.
//!
//! A rejected reload keeps the last good keymap and reports an `Event::Error`, so the user can
//! still fix the file.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use postio_config::change::ConfigChanged;
use postio_config::live::{LiveConfig, Reload};
use postio_config::paths::Platform;
use postio_config::validate::{Checked, Validation};
use postio_config::watch::{ConfigWatcher, WatchOptions};
use postio_config::{Config, ConfigError, KeyBindings, keys};

use crate::bridge::EventSink;
use crate::{ActionId, Context, ContextSet, Event, registry};

pub type ConfigChange = ConfigChanged;

pub(crate) static RESOLUTIONS: AtomicU64 = AtomicU64::new(0);

/// Bindings stay untyped strings: parsing them into GDK accelerators is the frontend's job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<ActionId, Vec<String>>,
    problems: Vec<String>,
    /// `None` for an empty keymap, which offers everything.
    platform: Option<Platform>,
}

impl Keymap {
    pub fn platform(&self) -> Option<Platform> {
        self.platform
    }

    /// An unusable override is a warning, never a failure: the command keeps its default or stays
    /// reachable from the palette.
    pub fn resolve(overrides: &KeyBindings) -> Self {
        Self::resolve_on(overrides, Platform::host())
    }

    /// A terminal delivers Ctrl, never Cmd, so `mod` is Ctrl there even on a Mac.
    pub fn for_terminal(overrides: &KeyBindings) -> Self {
        Self::resolve_on(overrides, Platform::Freedesktop)
    }

    /// Cached because resolution is quadratic in the number of commands and `MessageRowView` needs
    /// it per realised row (#1216). The unconfigured answer: anything reading `[keys]` uses
    /// `resolve`.
    pub fn defaults() -> &'static Self {
        static DEFAULTS: OnceLock<Keymap> = OnceLock::new();
        DEFAULTS.get_or_init(|| Self::resolve(&KeyBindings::default()))
    }

    /// `mod` is expanded here so lower layers see a concrete accelerator; `mod+k` and `ctrl+k` must
    /// conflict on Linux.
    pub fn resolve_on(overrides: &KeyBindings, platform: Platform) -> Self {
        RESOLUTIONS.fetch_add(1, Ordering::Relaxed);
        let mut keymap = Keymap {
            platform: Some(platform),
            ..Keymap::default()
        };
        for spec in registry::every_action() {
            keymap.bindings.insert(spec.id, Vec::new());
        }
        // Bind every parseable id even before its command exists, so a binding written before its
        // extension loads survives.
        for command in overrides.overrides().keys() {
            if let Ok(action) = command.parse::<ActionId>() {
                keymap.bindings.entry(action).or_default();
            }
        }

        // Two passes: an explicit `[keys]` entry outranks a built-in default, else registry order
        // would decide a contested key. Within each pass, registry order decides.
        for spec in keymap.known_actions() {
            let Some(binding) = overrides.overrides().get(spec.as_str()).cloned() else {
                continue;
            };
            if !registry::offered_on(spec, platform) {
                // Said, not dropped: the user would press the key and be ignored.
                keymap.problems.push(format!(
                    "`{spec}` is not offered on this platform, so `{binding}` does nothing here"
                ));
                continue;
            }
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
            // No key for a command this platform does not offer: it would claim the key and hold
            // `g` open as a prefix.
            if keymap.binding(spec.id).is_some() || !registry::offered_on(spec.id, platform) {
                continue;
            }
            // Nor for a default the platform keeps for another command
            // (`registry::binding_offered_on`).
            let Some(default) = spec
                .default_binding
                .filter(|binding| registry::binding_offered_on(spec.id, binding, platform))
                .map(|binding| keys::expand_mod(binding, platform))
            else {
                continue;
            };
            if let Some(taken) = keymap.holder_of(&default, spec.contexts) {
                // Palette-only rather than shadowing another key; said out loud so the loss is
                // visible.
                keymap.problems.push(format!(
                    "`{default}` is already bound to `{taken}`, so `{}` has no key \
                     and stays reachable from the palette",
                    spec.id
                ));
                continue;
            }
            keymap.claim(spec.id, default);
        }

        // Alternates last, only where nothing else wanted them: a second way must not cost another
        // command its first.
        for spec in registry::every_action() {
            if !registry::offered_on(spec.id, platform) {
                continue;
            }
            for alternate in spec.alternate_bindings {
                if !registry::binding_offered_on(spec.id, alternate, platform) {
                    continue;
                }
                let alternate = keys::expand_mod(alternate, platform);
                if keymap.holder_of(&alternate, spec.contexts).is_none() {
                    keymap.claim(spec.id, alternate);
                }
            }
        }

        for command in overrides.overrides().keys() {
            // An id that does not parse is reported; an unregistered extension id is the expected
            // startup state.
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

    /// Registered commands first in registry order, then ids `[keys]` named that nothing
    /// registered.
    fn known_actions(&self) -> Vec<ActionId> {
        self.bindings.keys().copied().collect()
    }

    /// `ContextSet::ANY` for an unregistered extension id: assume overlap, so a built-in cannot
    /// claim the key a user binding asked for.
    fn contexts_of(&self, action: ActionId) -> ContextSet {
        registry::spec(action)
            .map(|spec| spec.contexts)
            .unwrap_or(ContextSet::ANY)
    }

    pub fn binding(&self, command: impl Into<ActionId>) -> Option<&str> {
        self.bindings
            .get(&command.into())
            .and_then(|bindings| bindings.first())
            .map(String::as_str)
    }

    pub fn bindings(&self, command: impl Into<ActionId>) -> &[String] {
        self.bindings
            .get(&command.into())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn command_for(&self, context: Context, binding: &str) -> Option<ActionId> {
        registry::reachable(context)
            .find(|spec| {
                self.bindings(spec.id)
                    .iter()
                    .any(|candidate| candidate == binding)
            })
            .map(|spec| spec.id)
    }

    /// What the palette and cheat sheet filter on.
    pub fn offers(&self, action: impl Into<ActionId>) -> bool {
        let action = action.into();
        self.platform
            .is_none_or(|platform| registry::offered_on(action, platform))
    }

    pub fn problems(&self) -> &[String] {
        &self.problems
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigUpdate {
    pub reload: Reload,
    pub changed: ConfigChange,
    pub events: Vec<Event>,
}

impl ConfigUpdate {
    pub fn applied(&self) -> bool {
        self.reload == Reload::Applied
    }
}

#[derive(Debug, Clone)]
pub struct ConfigService {
    live: LiveConfig,
    keymap: Keymap,
}

impl ConfigService {
    pub fn new(path: &Path) -> Self {
        let live = LiveConfig::new(path);
        let keymap = Keymap::resolve(&live.config().keys);
        ConfigService { live, keymap }
    }

    /// A missing or broken file yields working defaults.
    pub fn load(path: &Path) -> Self {
        let live = LiveConfig::load(path);
        let keymap = Keymap::resolve(&live.config().keys);
        ConfigService { live, keymap }
    }

    pub fn config(&self) -> &Config {
        self.live.config()
    }

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn status(&self) -> &Validation {
        self.live.status()
    }

    pub fn path(&self) -> &Path {
        self.live.path()
    }

    /// Prefer `apply` with the watcher's `Checked`: parsing belongs off the UI thread.
    pub fn reload(&mut self) -> ConfigUpdate {
        let checked = postio_config::validate::check_path(self.live.path());
        self.apply(checked)
    }

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
                let message = self
                    .live
                    .status()
                    .first_error()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "the configuration could not be loaded".to_string());
                update.events.push(Event::Error {
                    message,
                    account: None,
                });
            }
            Reload::Unchanged => {}
            Reload::Applied => {
                update.changed = ConfigChange::between(&before, self.live.config());
                if update.changed.keys {
                    self.keymap = Keymap::resolve(&self.live.config().keys);
                    for problem in self.keymap.problems() {
                        update.events.push(Event::Error {
                            message: problem.clone(),
                            account: None,
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

/// The watcher parses on its own thread and applies through this handle.
#[derive(Debug, Clone)]
pub struct SharedConfig(Arc<Mutex<ConfigService>>);

impl SharedConfig {
    pub fn new(service: ConfigService) -> Self {
        SharedConfig(Arc::new(Mutex::new(service)))
    }

    pub fn load(path: &Path) -> Self {
        SharedConfig::new(ConfigService::load(path))
    }

    pub fn read<R>(&self, with: impl FnOnce(&ConfigService) -> R) -> R {
        with(&self.lock())
    }

    pub fn keymap(&self) -> Keymap {
        self.lock().keymap.clone()
    }

    pub fn apply(&self, checked: Checked, events: &EventSink) -> ConfigUpdate {
        let update = self.lock().apply(checked);
        for event in &update.events {
            events.emit(event.clone());
        }
        update
    }

    /// Watches the containing directory so it survives an editor's write-to-temp-and-rename.
    pub fn watch(&self, events: EventSink) -> Result<ConfigWatcher, ConfigError> {
        self.watch_with(WatchOptions::default(), events)
    }

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

    use crate::CommandId;

    #[test]
    fn a_terminal_binds_mod_to_ctrl_on_every_host() {
        let terminal = Keymap::for_terminal(&KeyBindings::default());
        assert_eq!(
            terminal.binding(CommandId::CommandPalette),
            Some("ctrl+k"),
            "a terminal never receives the command key"
        );
        assert_eq!(
            Keymap::resolve_on(&KeyBindings::default(), Platform::Apple)
                .binding(CommandId::CommandPalette),
            Some("cmd+k"),
            "the Mac app's own window does"
        );
    }

    #[test]
    fn a_command_the_platform_does_not_offer_has_no_key_there() {
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
            let offered = registry::offered_on(ActionId::Builtin(CommandId::NextScope), platform);
            assert_eq!(
                keymap.binding(CommandId::NextScope).is_some(),
                offered,
                "{platform:?}"
            );
            assert_eq!(
                keymap.offers(ActionId::Builtin(CommandId::NextScope)),
                offered
            );
        }
        let mac = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
        assert_eq!(mac.binding(CommandId::DetachComposer), None);
        assert!(
            mac.offers(ActionId::Builtin(CommandId::Archive)),
            "the rest are untouched"
        );
    }

    #[test]
    fn an_override_for_a_command_the_platform_does_not_offer_says_so() {
        let mut keys = KeyBindings::default();
        keys.overrides_mut()
            .insert("next_scope".to_owned(), "g x".to_owned());
        let mac = Keymap::resolve_on(&keys, Platform::Apple);
        assert_eq!(mac.binding(CommandId::NextScope), None);
        assert!(
            mac.problems()
                .iter()
                .any(|problem| problem.contains("next_scope")),
            "a `[keys]` entry that does nothing is said, not swallowed: {:?}",
            mac.problems()
        );
    }

    #[test]
    fn the_keymap_covers_every_command_in_the_registry() {
        let keymap = Keymap::resolve(&KeyBindings::default());
        for spec in registry::all() {
            if !registry::offered_on(ActionId::Builtin(spec.id), Platform::host()) {
                continue;
            }
            let first = spec
                .bindings()
                .find(|binding| {
                    registry::binding_offered_on(
                        ActionId::Builtin(spec.id),
                        binding,
                        Platform::host(),
                    )
                })
                .map(|binding| keys::expand_mod(binding, Platform::host()));
            assert_eq!(
                keymap.binding(spec.id),
                first.as_deref(),
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
