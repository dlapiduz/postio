//! A case that needs its own process (it sets a process-global) does not belong here; it has to
//! move back out.

mod config_doc;
mod live;
mod patch_spacing;
mod schema;
mod secrets;
mod validate;
mod validation_cost;
mod watch;
