//! One connection: read a command, change the world, answer.
//!
//! Everything a client can observe is decided here — including the things a
//! well-behaved server would never do. The quirks and faults are not bolted
//! on at the edges; they are branches inside the command they distort,
//! because that is where a real server's misbehaviour lives.

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use postio_model::FlagSet;

use super::state::{
    self, Mailbox, Message, flag_list, in_sequence_set, internal_date, sequence_set_of,
};
use super::wire::{Command, Conn, base64_decode, tokens, unquote, unwrap_parens};
use super::{Fault, Quirk, Shared};

/// The permanent flags every mailbox on this server accepts.
const FLAGS: &str = "\\Answered \\Flagged \\Deleted \\Seen \\Draft";

/// What one connection has selected.
#[derive(Clone, Debug)]
struct Selected {
    path: String,
    read_only: bool,
    /// The UIDs this connection last told the client about, in order. What is
    /// missing from the mailbox now is what it still owes an `EXPUNGE` for.
    known: Vec<u32>,
}

/// One connection's state machine.
pub(super) struct Session {
    conn: Conn,
    shared: Arc<Shared>,
    authenticated: bool,
    selected: Option<Selected>,
}

impl Session {
    pub(super) fn new(conn: Conn, shared: Arc<Shared>) -> Self {
        Self {
            conn,
            shared,
            authenticated: false,
            selected: None,
        }
    }

    /// Greets, then serves until the client leaves or the socket dies.
    pub(super) async fn run(mut self) -> io::Result<()> {
        let greeting = {
            let state = self.shared.lock();
            let advertised = if state.has(Quirk::AdvertiseExtensionsBeforeLogin) {
                state.capabilities.join(" ")
            } else {
                state.banner.join(" ")
            };
            format!("* OK [CAPABILITY {advertised}] postio test server ready")
        };
        self.conn.write_line(&greeting).await?;

        while let Some(command) = self.conn.read_command().await? {
            self.shared.lock().log.push(command.raw.clone());

            let fault = self.shared.lock().take_fault(&command.name);
            let trickling = matches!(fault, Some(Fault::Trickle { .. }));
            match fault {
                Some(Fault::DropConnection { .. }) => return self.tear(&command).await,
                Some(Fault::Stall { .. }) => return self.stall().await,
                Some(Fault::Trickle { gap, .. }) => self.conn.set_trickle(Some(gap)),
                Some(Fault::Reject { reason, .. }) => {
                    self.conn
                        .write_line(&format!("{} NO {reason}", command.tag))
                        .await?;
                    continue;
                }
                None => {}
            }

            let finished = self.dispatch(&command).await?;
            if trickling {
                self.conn.set_trickle(None);
            }
            if finished {
                break;
            }
        }

        Ok(())
    }

    /// Dispatches one command. `Ok(true)` ends the connection.
    async fn dispatch(&mut self, command: &Command) -> io::Result<bool> {
        let tag = command.tag.clone();

        match command.name.as_str() {
            "CAPABILITY" => {
                let advertised = {
                    let state = self.shared.lock();
                    if self.authenticated || state.has(Quirk::AdvertiseExtensionsBeforeLogin) {
                        state.capabilities.join(" ")
                    } else {
                        state.banner.join(" ")
                    }
                };
                self.conn
                    .write_line(&format!("* CAPABILITY {advertised}"))
                    .await?;
                self.ok(&tag, "CAPABILITY completed").await?;
            }
            "NOOP" | "CHECK" => {
                let updates = self.updates();
                self.conn.write(&updates).await?;
                self.ok(&tag, "NOOP completed").await?;
            }
            "ID" => {
                self.conn.write_line("* ID NIL").await?;
                self.ok(&tag, "ID completed").await?;
            }
            "LOGOUT" => {
                self.conn.write_line("* BYE logging out").await?;
                self.ok(&tag, "LOGOUT completed").await?;
                return Ok(true);
            }
            "LOGIN" => self.login(command).await?,
            "AUTHENTICATE" => self.authenticate(command).await?,
            "ENABLE" => self.enable(command).await?,
            "LIST" | "LSUB" => self.list(command).await?,
            "STATUS" => self.status(command).await?,
            "SELECT" | "EXAMINE" => self.select(command).await?,
            "CLOSE" | "UNSELECT" => {
                self.selected = None;
                self.ok(&tag, "mailbox closed").await?;
            }
            "FETCH" => self.fetch(command).await?,
            "STORE" => self.store(command).await?,
            "COPY" | "MOVE" => self.transfer(command).await?,
            "APPEND" => self.append(command).await?,
            "EXPUNGE" => self.expunge(command).await?,
            "IDLE" => self.idle(command).await?,
            other => {
                // Loudly, on purpose: a test server that guessed at a command
                // it does not implement would let a client's mistake pass.
                self.conn
                    .write_line(&format!("{tag} BAD {other} is not implemented here"))
                    .await?;
            }
        }

        Ok(false)
    }

    // -----------------------------------------------------------------
    // Session
    // -----------------------------------------------------------------

    async fn login(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let user = tokens.first().map(|token| command.text(token));
        let password = tokens.get(1).map(|token| command.text(token));
        let accepted = {
            let state = self.shared.lock();
            user.as_deref() == Some(state.account.as_str())
                && password.as_deref() == Some(state.password.as_str())
        };
        self.finish_auth(&command.tag, accepted).await
    }

    async fn authenticate(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let mechanism = tokens
            .first()
            .map(|token| token.to_ascii_uppercase())
            .unwrap_or_default();

        if mechanism != "PLAIN" {
            self.conn
                .write_line(&format!("{} NO only PLAIN is supported here", command.tag))
                .await?;
            return Ok(());
        }

        // With SASL-IR the credentials ride on the command line; without it
        // the server has to ask for them.
        let encoded = match tokens.get(1) {
            Some(initial) => initial.clone(),
            None => {
                self.conn.write_line("+ ").await?;
                match self.conn.read_line().await? {
                    Some(line) => String::from_utf8_lossy(&line).into_owned(),
                    None => return Ok(()),
                }
            }
        };

        let accepted = base64_decode(encoded.trim()).is_some_and(|decoded| {
            let mut fields = decoded.split(|byte| *byte == 0);
            let _authzid = fields.next();
            let authcid = fields.next().unwrap_or_default();
            let password = fields.next().unwrap_or_default();
            let state = self.shared.lock();
            authcid == state.account.as_bytes() && password == state.password.as_bytes()
        });

        self.finish_auth(&command.tag, accepted).await
    }

    async fn finish_auth(&mut self, tag: &str, accepted: bool) -> io::Result<()> {
        if !accepted {
            return self
                .conn
                .write_line(&format!(
                    "{tag} NO [AUTHENTICATIONFAILED] authentication failed"
                ))
                .await;
        }

        self.authenticated = true;
        let code = {
            let state = self.shared.lock();
            // Without this the client has to ask again — which is the whole
            // reason `ensure_capabilities` exists, and the path ADR 0001 Q3
            // says every session must take.
            if state.has(Quirk::CapabilityInLoginResponse) {
                format!("[CAPABILITY {}] ", state.capabilities.join(" "))
            } else {
                String::new()
            }
        };
        self.conn
            .write_line(&format!("{tag} OK {code}authenticated"))
            .await
    }

    async fn enable(&mut self, command: &Command) -> io::Result<()> {
        if !self.shared.lock().supports("ENABLE") {
            return self
                .conn
                .write_line(&format!("{} BAD ENABLE is not advertised", command.tag))
                .await;
        }

        let wanted: Vec<String> = command
            .tokens()
            .iter()
            .map(|token| token.to_ascii_uppercase())
            .collect();
        let enabled: Vec<String> = {
            let state = self.shared.lock();
            wanted
                .into_iter()
                .filter(|name| state.supports(name))
                .collect()
        };

        // RFC 5161 §3.1 requires the untagged echo. At least one mainstream
        // provider has shipped without it, which is why gating an extension
        // on the echo rather than on CAPABILITY is a bug.
        if !self.shared.lock().has(Quirk::OmitEnabledEcho) && !enabled.is_empty() {
            self.conn
                .write_line(&format!("* ENABLED {}", enabled.join(" ")))
                .await?;
        }
        self.ok(&command.tag, "ENABLE completed").await
    }

    // -----------------------------------------------------------------
    // Mailboxes
    // -----------------------------------------------------------------

    async fn list(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let pattern = tokens.get(1).map(|token| command.text(token));
        let subscribed_only = command.name == "LSUB";

        let rows: Vec<String> = {
            let state = self.shared.lock();
            state
                .mailboxes
                .iter()
                .filter(|mailbox| !subscribed_only || mailbox.subscribed)
                .filter(|mailbox| {
                    pattern
                        .as_deref()
                        .is_none_or(|pattern| matches(pattern, &mailbox.path))
                })
                .map(|mailbox| {
                    format!(
                        "* {} ({}) \"{}\" \"{}\"",
                        command.name,
                        mailbox.attributes.join(" "),
                        mailbox.delimiter,
                        mailbox.path
                    )
                })
                .collect()
        };

        for row in rows {
            self.conn.write_line(&row).await?;
        }
        self.ok(&command.tag, &format!("{} completed", command.name))
            .await
    }

    async fn status(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let path = tokens.first().map(|token| command.text(token));
        let wanted = tokens.get(1).cloned().unwrap_or_default();

        let row = {
            let state = self.shared.lock();
            path.as_deref()
                .and_then(|path| state.mailbox(path))
                .map(|mailbox| {
                    let items: Vec<String> = tokens_of(&wanted)
                        .iter()
                        .filter_map(|item| {
                            let value = match item.to_ascii_uppercase().as_str() {
                                "MESSAGES" => mailbox.messages.len() as u64,
                                "UIDNEXT" => u64::from(mailbox.uid_next),
                                "UIDVALIDITY" => u64::from(mailbox.uid_validity),
                                "UNSEEN" => u64::from(mailbox.unseen()),
                                "RECENT" => 0,
                                "HIGHESTMODSEQ" => mailbox.highest_mod_seq,
                                _ => return None,
                            };
                            Some(format!("{} {value}", item.to_ascii_uppercase()))
                        })
                        .collect();

                    format!("* STATUS \"{}\" ({})", mailbox.path, items.join(" "))
                })
        };

        let Some(row) = row else {
            return self.no(&command.tag, "no such mailbox").await;
        };
        self.conn.write_line(&row).await?;
        self.ok(&command.tag, "STATUS completed").await
    }

    async fn select(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let Some(path) = tokens.first().map(|token| command.text(token)) else {
            return self.bad(&command.tag, "SELECT needs a mailbox").await;
        };
        let parameters = tokens[1..].join(" ").to_ascii_uppercase();
        let read_only = command.name == "EXAMINE";

        let rendered = {
            let state = self.shared.lock();
            state.mailbox(&path).map(|mailbox| {
                let mut out: Vec<u8> = Vec::new();
                line(&mut out, &format!("* FLAGS ({FLAGS})"));
                line(
                    &mut out,
                    &format!("* OK [PERMANENTFLAGS ({FLAGS} \\*)] flags are permanent"),
                );
                line(&mut out, &format!("* {} EXISTS", mailbox.messages.len()));
                line(&mut out, "* 0 RECENT");
                line(
                    &mut out,
                    &format!("* OK [UIDVALIDITY {}] UIDs valid", mailbox.uid_validity),
                );
                line(
                    &mut out,
                    &format!("* OK [UIDNEXT {}] predicted next UID", mailbox.uid_next),
                );
                if state.supports("CONDSTORE") {
                    line(
                        &mut out,
                        &format!(
                            "* OK [HIGHESTMODSEQ {}] highest modification sequence",
                            mailbox.highest_mod_seq
                        ),
                    );
                }

                if let Some((uid_validity, mod_seq)) = qresync_parameters(&parameters) {
                    qresync_report(&mut out, mailbox, uid_validity, mod_seq);
                }
                out
            })
        };

        let Some(out) = rendered else {
            return self.no(&command.tag, "no such mailbox").await;
        };
        self.conn.write(&out).await?;
        self.selected = Some(Selected {
            known: self
                .shared
                .lock()
                .mailbox(&path)
                .map(Mailbox::uids)
                .unwrap_or_default(),
            path,
            read_only,
        });

        let access = if read_only { "READ-ONLY" } else { "READ-WRITE" };
        self.conn
            .write_line(&format!(
                "{} OK [{access}] {} completed",
                command.tag, command.name
            ))
            .await
    }

    // -----------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------

    async fn fetch(&mut self, command: &Command) -> io::Result<()> {
        let Some(selected) = self.selected.clone() else {
            return self.bad(&command.tag, "no mailbox is selected").await;
        };

        let tokens = command.tokens();
        let Some(set) = tokens.first().cloned() else {
            return self.bad(&command.tag, "FETCH needs a sequence set").await;
        };
        let items = tokens_of(tokens.get(1).map(String::as_str).unwrap_or(""));
        let changed_since = changed_since_of(&tokens[1..].join(" "));

        let Some(index) = self.shared.lock().index_of(&selected.path) else {
            return self.no(&command.tag, "the mailbox went away").await;
        };

        let updates = self.updates();
        let mut out = updates;

        {
            let mut state = self.shared.lock();
            let malformed = state.has(Quirk::MalformedFetchSequenceNumber);
            let condstore = state.supports("CONDSTORE");

            let mailbox = &state.mailboxes[index];
            let highest_uid = mailbox.highest_uid();
            let count = mailbox.messages.len() as u32;

            let chosen: Vec<(u32, Message)> = mailbox
                .messages
                .iter()
                .enumerate()
                .map(|(position, message)| (position as u32 + 1, message.clone()))
                .filter(|(sequence, message)| {
                    let (value, highest) = if command.uid {
                        (message.uid, highest_uid)
                    } else {
                        (*sequence, count)
                    };
                    in_sequence_set(&set, value, highest)
                })
                .filter(|(_, message)| changed_since.is_none_or(|floor| message.mod_seq > floor))
                .collect();

            // A resync pull is exactly where this provider has been seen to
            // emit a sequence number no decoder can read. `io-imap` skips the
            // line and completes the command `Ok`, so the pull looks whole
            // while a message's flags never arrived.
            if malformed && changed_since.is_some() {
                line(&mut out, "* -1 FETCH (UID 1 FLAGS (\\Seen))");
            }

            let mut seen: Vec<u32> = Vec::new();
            for (sequence, message) in &chosen {
                let rendered = render_fetch(
                    message,
                    *sequence,
                    &items,
                    command.uid,
                    condstore,
                    &mut seen,
                );
                out.extend_from_slice(&rendered);
            }

            // A fetch that is not a `.PEEK` sets `\Seen`, which is the whole
            // reason every fetch Postio issues peeks.
            if !seen.is_empty() {
                let mailbox = &mut state.mailboxes[index];
                let mod_seq = mailbox.bump();
                for uid in seen {
                    if let Some(message) = mailbox.find_mut(uid) {
                        message.flags.insert(postio_model::Flag::Seen);
                        message.mod_seq = mod_seq;
                    }
                }
            }
        }

        self.conn.write(&out).await?;
        self.ok(&command.tag, "FETCH completed").await
    }

    async fn store(&mut self, command: &Command) -> io::Result<()> {
        let Some(selected) = self.selected.clone() else {
            return self.bad(&command.tag, "no mailbox is selected").await;
        };
        if selected.read_only {
            return self.no(&command.tag, "the mailbox is read-only").await;
        }

        let tokens = command.tokens();
        let (Some(set), Some(operation)) = (tokens.first().cloned(), tokens.get(1).cloned()) else {
            return self
                .bad(&command.tag, "STORE needs a set and an operation")
                .await;
        };
        let operation = operation.to_ascii_uppercase();
        let silent = operation.ends_with(".SILENT");
        let wanted: FlagSet = tokens_of(tokens.get(2).map(String::as_str).unwrap_or(""))
            .iter()
            .map(|name| state::flag(name))
            .collect();

        let Some(index) = self.shared.lock().index_of(&selected.path) else {
            return self.no(&command.tag, "the mailbox went away").await;
        };

        let mut out: Vec<u8> = Vec::new();
        {
            let mut state = self.shared.lock();
            let condstore = state.supports("CONDSTORE");
            let mailbox = &mut state.mailboxes[index];
            let highest_uid = mailbox.highest_uid();
            let count = mailbox.messages.len() as u32;
            let mod_seq = mailbox.bump();

            let mut touched: Vec<(u32, u32)> = Vec::new();
            for (position, message) in mailbox.messages.iter_mut().enumerate() {
                let sequence = position as u32 + 1;
                let (value, highest) = if command.uid {
                    (message.uid, highest_uid)
                } else {
                    (sequence, count)
                };
                if !in_sequence_set(&set, value, highest) {
                    continue;
                }

                message.flags = if operation.starts_with('+') {
                    message
                        .flags
                        .iter()
                        .cloned()
                        .chain(wanted.iter().cloned())
                        .collect()
                } else if operation.starts_with('-') {
                    message
                        .flags
                        .iter()
                        .filter(|flag| !wanted.contains(flag))
                        .cloned()
                        .collect()
                } else {
                    wanted.clone()
                };
                message.mod_seq = mod_seq;
                touched.push((sequence, message.uid));
            }

            if !silent {
                for (sequence, uid) in touched {
                    let message = mailbox.find(uid).expect("just touched");
                    let mut items = format!("UID {uid} FLAGS ({})", flag_list(&message.flags));
                    if condstore {
                        items.push_str(&format!(" MODSEQ ({})", message.mod_seq));
                    }
                    line(&mut out, &format!("* {sequence} FETCH ({items})"));
                }
            }
        }

        self.conn.write(&out).await?;
        self.shared.notify();
        self.ok(&command.tag, "STORE completed").await
    }

    async fn transfer(&mut self, command: &Command) -> io::Result<()> {
        let Some(selected) = self.selected.clone() else {
            return self.bad(&command.tag, "no mailbox is selected").await;
        };
        let tokens = command.tokens();
        let (Some(set), Some(destination)) = (
            tokens.first().cloned(),
            tokens.get(1).map(|token| command.text(token)),
        ) else {
            return self
                .bad(&command.tag, "COPY needs a set and a mailbox")
                .await;
        };

        let moving = command.name == "MOVE";
        let indices = {
            let state = self.shared.lock();
            state
                .index_of(&selected.path)
                .zip(state.index_of(&destination))
        };
        let Some((source, target)) = indices else {
            return self
                .no(&command.tag, "[TRYCREATE] no such destination mailbox")
                .await;
        };

        let mut out: Vec<u8> = Vec::new();
        let outcome = {
            let mut state = self.shared.lock();
            let uid_plus = state.supports("UIDPLUS");

            let mailbox = &state.mailboxes[source];
            let highest_uid = mailbox.highest_uid();
            let count = mailbox.messages.len() as u32;
            let chosen: Vec<Message> = mailbox
                .messages
                .iter()
                .enumerate()
                .filter(|(position, message)| {
                    let (value, highest) = if command.uid {
                        (message.uid, highest_uid)
                    } else {
                        (*position as u32 + 1, count)
                    };
                    in_sequence_set(&set, value, highest)
                })
                .map(|(_, message)| message.clone())
                .collect();

            let source_uid_validity = state.mailboxes[source].uid_validity;
            let mut sources: BTreeSet<u32> = BTreeSet::new();
            let mut destinations: BTreeSet<u32> = BTreeSet::new();
            for message in &chosen {
                let landed = state.mailboxes[target].push(
                    message.raw.clone(),
                    message.flags.clone(),
                    message.internal_date,
                );
                sources.insert(message.uid);
                destinations.insert(landed);
            }
            let destination_uid_validity = state.mailboxes[target].uid_validity;

            if moving {
                for uid in &sources {
                    let position = state.mailboxes[source]
                        .messages
                        .iter()
                        .position(|message| message.uid == *uid);
                    if let Some(position) = position {
                        line(&mut out, &format!("* {} EXPUNGE", position + 1));
                    }
                    state.mailboxes[source].remove(*uid);
                }
            }

            uid_plus
                .then(|| {
                    format!(
                        "[COPYUID {destination_uid_validity} {} {}]",
                        sequence_set_of(&sources),
                        sequence_set_of(&destinations),
                    )
                })
                .map(|code| (code, source_uid_validity))
        };

        self.conn.write(&out).await?;
        self.shared.notify();
        if let Some(selected) = self.selected.as_mut() {
            selected.known = self
                .shared
                .lock()
                .mailbox(&selected.path)
                .map(Mailbox::uids)
                .unwrap_or_default();
        }

        let code = outcome
            .map(|(code, _)| format!("{code} "))
            .unwrap_or_default();
        self.conn
            .write_line(&format!(
                "{} OK {code}{} completed",
                command.tag, command.name
            ))
            .await
    }

    async fn append(&mut self, command: &Command) -> io::Result<()> {
        let tokens = command.tokens();
        let Some(path) = tokens.first().map(|token| command.text(token)) else {
            return self.bad(&command.tag, "APPEND needs a mailbox").await;
        };
        let Some(raw) = tokens.last().map(|token| command.bytes(token)) else {
            return self.bad(&command.tag, "APPEND needs a message").await;
        };
        let flags: FlagSet = tokens
            .iter()
            .find(|token| token.starts_with('('))
            .map(|token| {
                tokens_of(token)
                    .iter()
                    .map(|name| state::flag(name))
                    .collect()
            })
            .unwrap_or_default();

        let outcome = {
            let mut state = self.shared.lock();
            let uid_plus = state.supports("UIDPLUS");
            state.mailbox_mut(&path).map(|mailbox| {
                let uid = mailbox.push(raw, flags, Utc::now());
                let uid_validity = mailbox.uid_validity;
                uid_plus.then(|| format!("[APPENDUID {uid_validity} {uid}] "))
            })
        };
        let Some(outcome) = outcome else {
            return self.no(&command.tag, "[TRYCREATE] no such mailbox").await;
        };

        self.shared.notify();
        self.conn
            .write_line(&format!(
                "{} OK {}APPEND completed",
                command.tag,
                outcome.unwrap_or_default()
            ))
            .await
    }

    async fn expunge(&mut self, command: &Command) -> io::Result<()> {
        let Some(selected) = self.selected.clone() else {
            return self.bad(&command.tag, "no mailbox is selected").await;
        };
        if selected.read_only {
            return self.no(&command.tag, "the mailbox is read-only").await;
        }
        let set = command
            .uid
            .then(|| command.tokens().first().cloned())
            .flatten();

        let Some(index) = self.shared.lock().index_of(&selected.path) else {
            return self.no(&command.tag, "the mailbox went away").await;
        };

        let mut out: Vec<u8> = Vec::new();
        {
            let mut state = self.shared.lock();
            let mailbox = &mut state.mailboxes[index];
            let highest_uid = mailbox.highest_uid();

            let doomed: Vec<u32> = mailbox
                .messages
                .iter()
                .filter(|message| message.flags.is_deleted())
                .filter(|message| {
                    set.as_deref()
                        .is_none_or(|set| in_sequence_set(set, message.uid, highest_uid))
                })
                .map(|message| message.uid)
                .collect();

            // Descending, so each sequence number is still valid when the
            // client applies it — RFC 3501 §7.4.1.
            for uid in doomed.iter().rev() {
                if let Some(position) = mailbox
                    .messages
                    .iter()
                    .position(|message| message.uid == *uid)
                {
                    line(&mut out, &format!("* {} EXPUNGE", position + 1));
                }
                mailbox.remove(*uid);
            }
        }

        self.conn.write(&out).await?;
        self.shared.notify();
        if let Some(selected) = self.selected.as_mut() {
            selected.known = self
                .shared
                .lock()
                .mailbox(&selected.path)
                .map(Mailbox::uids)
                .unwrap_or_default();
        }
        self.ok(&command.tag, "EXPUNGE completed").await
    }

    // -----------------------------------------------------------------
    // IDLE
    // -----------------------------------------------------------------

    async fn idle(&mut self, command: &Command) -> io::Result<()> {
        if self.selected.is_none() {
            return self.bad(&command.tag, "no mailbox is selected").await;
        }
        self.conn.write_line("+ idling").await?;

        loop {
            // Subscribe before looking, so a change landing between the two
            // is not slept through.
            let shared = Arc::clone(&self.shared);
            let notified = shared.notified();
            tokio::pin!(notified);

            let updates = self.updates();
            if !updates.is_empty() {
                self.conn.write(&updates).await?;
            }

            tokio::select! {
                line = self.conn.read_line() => {
                    match line? {
                        None => return Ok(()),
                        Some(line) => {
                            if String::from_utf8_lossy(&line).trim().eq_ignore_ascii_case("DONE") {
                                break;
                            }
                        }
                    }
                }
                _ = &mut notified => continue,
            }
        }

        self.ok(&command.tag, "IDLE terminated").await
    }

    // -----------------------------------------------------------------
    // Faults
    // -----------------------------------------------------------------

    /// Answers half a response and hangs up, the way a connection reset in
    /// the middle of a body download looks to a client.
    async fn tear(&mut self, command: &Command) -> io::Result<()> {
        if command.name == "FETCH" {
            self.conn
                .write(b"* 1 FETCH (UID 1 BODY[] {65536}\r\n")
                .await?;
            self.conn
                .write(b"Subject: torn\r\n\r\nthe rest of th")
                .await?;
        }
        Ok(())
    }

    /// Accepts the command and never answers it.
    async fn stall(&mut self) -> io::Result<()> {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    // -----------------------------------------------------------------
    // Plumbing
    // -----------------------------------------------------------------

    /// The untagged lines this connection still owes the client: what
    /// vanished from the selected mailbox, and how many messages there are
    /// now.
    fn updates(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let Some(selected) = self.selected.as_mut() else {
            return out;
        };

        let state = self.shared.lock();
        let Some(mailbox) = state.mailbox(&selected.path) else {
            return out;
        };
        let now = mailbox.uids();
        if now == selected.known {
            return out;
        }

        let gone: Vec<u32> = selected
            .known
            .iter()
            .copied()
            .filter(|uid| !now.contains(uid))
            .collect();
        let mut known = selected.known.clone();
        for uid in gone {
            if let Some(position) = known.iter().position(|candidate| *candidate == uid) {
                line(&mut out, &format!("* {} EXPUNGE", position + 1));
                known.remove(position);
            }
        }
        if now.len() != known.len() {
            line(&mut out, &format!("* {} EXISTS", now.len()));
        }

        selected.known = now;
        out
    }

    async fn ok(&mut self, tag: &str, text: &str) -> io::Result<()> {
        self.conn.write_line(&format!("{tag} OK {text}")).await
    }

    async fn no(&mut self, tag: &str, text: &str) -> io::Result<()> {
        self.conn.write_line(&format!("{tag} NO {text}")).await
    }

    async fn bad(&mut self, tag: &str, text: &str) -> io::Result<()> {
        self.conn.write_line(&format!("{tag} BAD {text}")).await
    }
}

// ---------------------------------------------------------------------------
// Response building
// ---------------------------------------------------------------------------

fn line(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\r\n");
}

/// Renders one `* n FETCH (…)` line.
///
/// `seen` collects the UIDs a non-peeking body fetch has just made `\Seen`.
fn render_fetch(
    message: &Message,
    sequence: u32,
    items: &[String],
    uid_command: bool,
    condstore: bool,
    seen: &mut Vec<u32>,
) -> Vec<u8> {
    let parsed = super::mime::parse(&message.raw);
    let mut out: Vec<u8> = Vec::new();
    let mut rendered: Vec<Vec<u8>> = Vec::new();

    // RFC 3501 §6.4.8: a UID FETCH always reports the UID, asked for or not.
    if uid_command && !items.iter().any(|item| item.eq_ignore_ascii_case("UID")) {
        rendered.push(format!("UID {}", message.uid).into_bytes());
    }

    for item in items {
        let upper = item.to_ascii_uppercase();
        let piece: Vec<u8> = match upper.as_str() {
            "UID" => format!("UID {}", message.uid).into_bytes(),
            "FLAGS" => format!("FLAGS ({})", flag_list(&message.flags)).into_bytes(),
            "INTERNALDATE" => {
                format!("INTERNALDATE \"{}\"", internal_date(message.internal_date)).into_bytes()
            }
            "RFC822.SIZE" => format!("RFC822.SIZE {}", message.raw.len()).into_bytes(),
            "ENVELOPE" => {
                format!("ENVELOPE {}", super::mime::envelope(&message.raw, &parsed)).into_bytes()
            }
            "BODYSTRUCTURE" | "BODY" => format!(
                "{upper} {}",
                super::mime::body_structure(&message.raw, &parsed)
            )
            .into_bytes(),
            "MODSEQ" if condstore => format!("MODSEQ ({})", message.mod_seq).into_bytes(),
            "MODSEQ" => continue,
            _ if upper.starts_with("BODY") => {
                let (spec, partial, peek) = body_item(item);
                if !peek {
                    seen.push(message.uid);
                }
                let mut bytes =
                    super::mime::section(&message.raw, &parsed, &spec).unwrap_or_default();
                let mut label = format!("BODY[{spec}]");
                if let Some((offset, length)) = partial {
                    let start = (offset as usize).min(bytes.len());
                    let end = (start + length as usize).min(bytes.len());
                    bytes = bytes[start..end].to_vec();
                    label.push_str(&format!("<{offset}>"));
                }
                let mut piece = format!("{label} {{{}}}\r\n", bytes.len()).into_bytes();
                piece.extend_from_slice(&bytes);
                piece
            }
            _ => continue,
        };
        rendered.push(piece);
    }

    out.extend_from_slice(format!("* {sequence} FETCH (").as_bytes());
    for (index, piece) in rendered.iter().enumerate() {
        if index > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(piece);
    }
    out.extend_from_slice(b")\r\n");
    out
}

/// Splits `BODY.PEEK[1.2]<0.4096>` into its section, its window and whether
/// it peeks.
fn body_item(item: &str) -> (String, Option<(u32, u32)>, bool) {
    let peek = item.to_ascii_uppercase().starts_with("BODY.PEEK");
    let (spec, rest) = match (item.find('['), item.find(']')) {
        (Some(open), Some(close)) if close > open => {
            (item[open + 1..close].to_owned(), &item[close + 1..])
        }
        _ => (String::new(), ""),
    };

    let partial = rest
        .trim()
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .and_then(|window| window.split_once('.'))
        .and_then(|(offset, length)| Some((offset.parse().ok()?, length.parse().ok()?)));

    (spec, partial, peek)
}

/// The `CHANGEDSINCE n` of a fetch modifier group, if there is one.
fn changed_since_of(text: &str) -> Option<u64> {
    let upper = text.to_ascii_uppercase();
    let at = upper.find("CHANGEDSINCE")?;
    upper[at + "CHANGEDSINCE".len()..]
        .split(|character: char| !character.is_ascii_digit())
        .find(|piece| !piece.is_empty())?
        .parse()
        .ok()
}

/// The `(QRESYNC (uidvalidity modseq …))` of a select parameter group.
fn qresync_parameters(text: &str) -> Option<(u32, u64)> {
    let at = text.find("QRESYNC")?;
    let numbers: Vec<u64> = text[at..]
        .split(|character: char| !character.is_ascii_digit())
        .filter(|piece| !piece.is_empty())
        .filter_map(|piece| piece.parse().ok())
        .collect();
    match numbers.as_slice() {
        [uid_validity, mod_seq, ..] => Some((*uid_validity as u32, *mod_seq)),
        _ => None,
    }
}

/// The `VANISHED (EARLIER …)` and implicit FETCHes a QRESYNC select owes a
/// client that was last here at `mod_seq`.
fn qresync_report(out: &mut Vec<u8>, mailbox: &Mailbox, uid_validity: u32, mod_seq: u64) {
    if mailbox.uid_validity != uid_validity {
        // The UID space was renumbered: nothing the client holds means
        // anything, and RFC 7162 §3.2.5.2 says to say nothing rather than
        // report deltas against a dead generation.
        return;
    }

    let vanished: BTreeSet<u32> = mailbox
        .vanished
        .iter()
        .filter(|(_, at)| *at > mod_seq)
        .map(|(uid, _)| *uid)
        .collect();
    if !vanished.is_empty() {
        line(
            out,
            &format!("* VANISHED (EARLIER) {}", sequence_set_of(&vanished)),
        );
    }

    for (position, message) in mailbox.messages.iter().enumerate() {
        if message.mod_seq <= mod_seq {
            continue;
        }
        line(
            out,
            &format!(
                "* {} FETCH (UID {} FLAGS ({}) MODSEQ ({}))",
                position + 1,
                message.uid,
                flag_list(&message.flags),
                message.mod_seq
            ),
        );
    }
}

/// Splits a parenthesised list into its items.
fn tokens_of(group: &str) -> Vec<String> {
    tokens(unwrap_parens(group.trim()))
        .into_iter()
        .map(|token| unquote(&token))
        .collect()
}

/// Whether an RFC 3501 list pattern matches a mailbox path.
fn matches(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() || pattern == "*" || pattern == "%" {
        return true;
    }
    match pattern.split_once('*') {
        Some((prefix, suffix)) => path.starts_with(prefix) && path.ends_with(suffix),
        None => pattern.eq_ignore_ascii_case(path),
    }
}

/// Kept honest by the module's own tests rather than by a client.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changedsince_modifier_is_found_wherever_it_sits() {
        assert_eq!(changed_since_of("(CHANGEDSINCE 900)"), Some(900));
        assert_eq!(changed_since_of("(CHANGEDSINCE 12 VANISHED)"), Some(12));
        assert_eq!(changed_since_of("(FLAGS)"), None);
    }

    #[test]
    fn a_qresync_select_parameter_yields_its_generation_and_sequence() {
        assert_eq!(
            qresync_parameters("(QRESYNC (4242 900))"),
            Some((4_242, 900))
        );
        assert_eq!(qresync_parameters("(CONDSTORE)"), None);
    }

    #[test]
    fn a_body_item_splits_into_section_window_and_peek() {
        assert_eq!(
            body_item("BODY.PEEK[]<0.4096>"),
            (String::new(), Some((0, 4096)), true)
        );
        assert_eq!(body_item("BODY[1.2]"), ("1.2".to_owned(), None, false));
        assert_eq!(
            body_item("BODY.PEEK[HEADER.FIELDS (REFERENCES)]"),
            ("HEADER.FIELDS (REFERENCES)".to_owned(), None, true)
        );
    }

    #[test]
    fn a_list_pattern_matches_by_prefix_and_suffix() {
        assert!(matches("*", "INBOX"));
        assert!(matches("Projects/*", "Projects/Postio"));
        assert!(!matches("Projects/*", "INBOX"));
        assert!(matches("INBOX", "inbox"));
    }
}
