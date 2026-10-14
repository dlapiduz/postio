//! Postio over the Model Context Protocol (ADR 0010).
//!
//! An AI agent -- Claude Code, Claude Desktop, any MCP client -- launches
//! `postio-mcp` as a subprocess and speaks JSON-RPC to it on stdin and
//! stdout. There is no socket: nothing another user or another machine can
//! reach (ADR 0010 Q6).
//!
//! # A frontend, not a side door
//!
//! Every read and every write go through `postio-client`, served by a
//! `postio-host` in this process, exactly as the terminal's and the desktop
//! app's do (ADR 0010 Q2, ADR 0041). Nothing here opens the database on its
//! own terms. One app has the store at a time, so the store is opened for a
//! tool call and closed after it ([`backend::Installed`]): the agent does not
//! keep the desktop app out for as long as its session lasts, and a call
//! made while the desktop app is open is told so.
//!
//! The server starts no sync and fetches nothing. It answers from what
//! Postio already holds on this computer, and makes no connection of its own.
//!
//! # What an agent can do
//!
//! Read what the person has shared in `[mcp]` (see [`scope`]), and write a
//! draft: a draft is local, and mail leaves only when the person opens it in
//! Postio and sends it. Where `[mcp]`'s `actions` allow it, archive, move
//! between shared folders and label (ADR 0010, amended 2026-10-14), each by
//! the command the apps send for the same gesture. There is no send,
//! forward, delete or mark-read tool, absent rather than gated (ADR 0010
//! Q4), and a move into the Trash or Junk is refused as the delete it is.
//!
//! # Mail is untrusted
//!
//! Every byte of mail a tool returns was written by whoever sent it. Bodies
//! come back as the text of `postio-body`'s document, never as the raw
//! `text/html` part; mail-derived text travels in its own content block,
//! labelled as data, never inside a sentence the server wrote; and the tool
//! names, descriptions and schemas are literals that nothing derived from
//! mail can reach ([`tools`], ADR 0010 Q5).

pub mod audit;
pub mod backend;
pub mod scope;
pub mod server;
pub mod tools;

pub use audit::Audit;
pub use backend::{Backend, Held, Installed, Lease};
pub use scope::Scope;
pub use server::{Server, serve};
