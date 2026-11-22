//! POSTIO-OWN-BINARY: the command registry is a process-global, append-only table.

use std::sync::atomic::{AtomicU32, Ordering};

use postio_core::registry::{self, ExtCommand, RegistrationError};
use postio_core::{ActionId, CommandId, Context, ContextSet, Keymap, Recovery};

/// Unique per test: the registry is process-global and append-only, so a shared id would collide on
/// the duplicate check.
fn unique_id(name: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!("test{}:{name}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn bindings(overrides: &[(&str, &str)]) -> postio_config::KeyBindings {
    let mut keys = postio_config::KeyBindings::default();
    for (command, binding) in overrides {
        keys.overrides_mut()
            .insert((*command).to_string(), (*binding).to_string());
    }
    keys
}

fn command(id: &str, title: &str) -> ExtCommand {
    ExtCommand {
        id: id.to_string(),
        title: title.to_string(),
        default_binding: None,
        alternate_bindings: Vec::new(),
        contexts: ContextSet::from_slice(&[Context::List]),
        destructive: false,
        recovery: Recovery::None,
    }
}

#[test]
fn an_extension_command_registers_at_runtime() {
    let id = unique_id("summarise-thread");
    let ext = registry::register(command(&id, "Summarise thread")).expect("it registers");

    assert_eq!(
        ext.as_str(),
        id,
        "the id round-trips as the string it was given"
    );
    let spec = registry::spec(ActionId::Ext(ext)).expect("a spec for what was just registered");
    assert_eq!(spec.title, "Summarise thread");
    assert_eq!(spec.id, ActionId::Ext(ext));
}

#[test]
fn it_reaches_the_palette_and_the_cheat_sheet() {
    let id = unique_id("file-to-receipts");
    let ext = registry::register(command(&id, "File to receipts")).expect("it registers");

    let reachable: Vec<ActionId> = registry::reachable(Context::List)
        .map(|spec| spec.id)
        .collect();

    assert!(
        reachable.contains(&ActionId::Ext(ext)),
        "the extension command is not reachable in the list context, so it \
         appears in neither Ctrl+K nor `?`"
    );
    assert!(
        reachable.contains(&ActionId::Builtin(CommandId::Archive)),
        "and the built-ins are still there beside it, on the same footing"
    );

    let composing: Vec<ActionId> = registry::reachable(Context::Composer)
        .map(|spec| spec.id)
        .collect();
    assert!(
        !composing.contains(&ActionId::Ext(ext)),
        "it was registered for the list only and turned up in the composer"
    );
}

#[test]
fn it_is_bindable_from_keys_by_its_namespaced_id() {
    let id = unique_id("triage");
    let ext = registry::register(ExtCommand {
        default_binding: Some("ctrl+shift+t".to_string()),
        ..command(&id, "Triage")
    })
    .expect("it registers");

    let keymap = Keymap::resolve(&Default::default());
    assert_eq!(
        keymap.binding(ActionId::Ext(ext)),
        Some("ctrl+shift+t"),
        "its own default binding did not survive resolution"
    );
    assert_eq!(
        keymap.command_for(Context::List, "ctrl+shift+t"),
        Some(ActionId::Ext(ext)),
        "the key does not reach the command it is bound to"
    );
}

#[test]
fn a_destructive_extension_command_without_recovery_is_rejected() {
    let error = registry::register(ExtCommand {
        destructive: true,
        recovery: Recovery::None,
        ..command(&unique_id("shred"), "Shred")
    })
    .expect_err("a destructive command with no way back must not register");

    assert!(matches!(error, RegistrationError::UnrecoverableDestructive));

    registry::register(ExtCommand {
        destructive: true,
        recovery: Recovery::Undo,
        ..command(&unique_id("shred"), "Shred")
    })
    .expect("destructive with undo is a legitimate command");
}

#[test]
fn an_id_must_be_namespaced_and_must_not_shadow_a_built_in() {
    assert!(matches!(
        registry::register(command("summarise", "No namespace")),
        Err(RegistrationError::NotNamespaced)
    ));
    assert!(matches!(
        registry::register(command("archive", "Shadowing a built-in")),
        Err(RegistrationError::NotNamespaced)
    ));

    let id = unique_id("twice");
    registry::register(command(&id, "First")).expect("the first registration wins");
    assert!(
        matches!(
            registry::register(command(&id, "Second")),
            Err(RegistrationError::AlreadyRegistered)
        ),
        "two commands with one id is the same wiring bug as two handlers for \
         one id, and is much cheaper to find at registration"
    );
}

#[test]
fn a_binding_written_before_the_command_registers_still_finds_it() {
    let id = unique_id("late");
    let overrides = bindings(&[(id.as_str(), "ctrl+alt+l")]);

    let keymap = Keymap::resolve(&overrides);
    let action: ActionId = id
        .parse()
        .expect("a namespaced id parses whether or not it exists");
    assert_eq!(
        keymap.binding(action),
        Some("ctrl+alt+l"),
        "the binding was dropped because the command did not exist yet"
    );

    let ext = registry::register(command(&id, "Late arrival")).expect("it registers");
    assert_eq!(action, ActionId::Ext(ext));
    assert_eq!(
        keymap.command_for(Context::List, "ctrl+alt+l"),
        Some(action),
        "the key does not reach the command now that it exists, so the \
         binding written before it loaded is dead"
    );
}

#[test]
fn an_unknown_built_in_id_is_still_a_problem_but_an_unregistered_extension_is_not() {
    let misspelt = bindings(&[("arhcive", "q")]);
    assert!(
        Keymap::resolve(&misspelt)
            .problems()
            .iter()
            .any(|problem| problem.contains("arhcive")),
        "a misspelt built-in should still be reported"
    );

    let pending = bindings(&[("mcp:not-loaded-yet", "q")]);
    assert!(
        !Keymap::resolve(&pending)
            .problems()
            .iter()
            .any(|problem| problem.contains("mcp:not-loaded-yet")),
        "an extension that has not registered yet is not a configuration error"
    );
}

#[test]
fn a_registered_command_can_actually_run() {
    let id = unique_id("run-me");
    let ext = registry::register(command(&id, "Run me")).expect("it registers");

    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dispatcher = postio_core::Dispatcher::builder()
        .on_ext(ext, {
            let ran = ran.clone();
            move |invocation: postio_core::dispatch::ExtInvocation| {
                let ran = ran.clone();
                async move {
                    assert_eq!(invocation.id(), ext);
                    ran.store(true, Ordering::Relaxed);
                    invocation.emit(postio_core::Event::Error {
                        message: "ran".to_string(),
                        account: None,
                    });
                    Ok(())
                }
            }
        })
        .build();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    let (sink, events) = postio_core::bridge::event_channel();
    runtime.block_on(dispatcher.dispatch_ext(ext, sink));

    assert!(ran.load(Ordering::Relaxed), "the handler never ran");
    assert!(matches!(
        events.try_next(),
        Some(postio_core::Event::Error { message, .. }) if message == "ran"
    ));
}

#[test]
fn an_unhandled_extension_command_is_rejected_rather_than_silent() {
    let id = unique_id("no-handler");
    let ext = registry::register(command(&id, "No handler")).expect("it registers");

    let dispatcher = postio_core::Dispatcher::builder().build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    let (sink, events) = postio_core::bridge::event_channel();
    runtime.block_on(dispatcher.dispatch_ext(ext, sink));

    match events.try_next() {
        Some(postio_core::Event::CommandRejected { command, reason }) => {
            assert_eq!(command, ActionId::Ext(ext));
            assert!(
                reason.contains("No handler"),
                "the reason names it: {reason}"
            );
        }
        other => panic!("expected a rejection naming the command, got {other:?}"),
    }
}

#[test]
fn the_built_in_table_is_unchanged() {
    let built_ins: Vec<CommandId> = registry::all().map(|spec| spec.id).collect();
    assert_eq!(built_ins.len(), CommandId::ALL.len());

    let spec: &'static postio_core::CommandSpec = registry::get(CommandId::Archive);
    assert_eq!(spec.id, CommandId::Archive);
    let _: &'static str = spec.title;
}
