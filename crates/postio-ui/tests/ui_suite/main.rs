//! A case that sets a process-global (`set_var`, crate attributes) does not belong in this binary:
//! it would change what its neighbours see.

mod calendar_agenda;
mod hostile_mail;
mod keybindings_doc;
mod keymap_api;
mod keymap_defaults;
mod keymap_live;
mod keymap_sheet;
mod one_keymap;
mod reader_tokens;
mod remote_blocked_document;
mod sender_authenticity;
mod tokens;
