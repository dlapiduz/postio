//! Cases that call `registry::register` or read `every_action()` keep their own binaries: in one
//! process they race on the process-global registry. A case that sets a process-global does not
//! belong here.

mod bridge;
mod bridge_runtime_handle;
mod command_registry;
mod config;
mod dispatch;
mod event_hub;
mod events;
mod one_keymap;
mod perf_budget;
mod platform_bindings;
mod selection;
mod state;
mod undo;
