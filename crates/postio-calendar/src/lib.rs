//! Calendar invitations carried in mail (`specs/007-postio-focus`,
//! research R9).
//!
//! It parses a `text/calendar` part into an invitation and writes the reply
//! that answers one (RFC 5545 and RFC 5546). It is a pure leaf: no store, no
//! toolkit, no network.
