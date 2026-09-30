//! The first run: an address, its servers and a password, then how far back
//! to sync (US7).
//!
//! The desktop's first-run screen, in a terminal: the same three steps, the
//! same states (`postio_ui::onboarding::Status`), the same sentences. What is
//! typed is kept here; finding the servers and proving the password are the
//! host's (`Req::Discover`, `Req::AddAccount`), which do it as the desktop
//! does.

use crossterm::event::{KeyCode, KeyEvent};
use postio_ui::onboarding::{Server, Settings, Status, Submission, SyncWindow};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

/// Which field the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The address.
    Address,
    /// The name to send as.
    Name,
    /// The password.
    Password,
    /// The incoming server, when it has to be typed.
    Incoming,
    /// The outgoing server, when it has to be typed.
    Outgoing,
    /// The OAuth client's id, for a provider that signs in in a browser.
    ClientId,
    /// Its secret, when the provider issued one.
    ClientSecret,
}

/// What a key in the first run asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Nothing beyond a redraw.
    Nothing,
    /// Look up this address's servers.
    Discover(String),
    /// Prove and save this account.
    Add(Box<Submission>),
    /// Sync this far back, and start.
    Start(SyncWindow),
    /// Begin a browser sign-in.
    BeginOAuth(Box<Submission>),
    /// Open the consent URL, the person having asked.
    Open(String),
    /// Copy the consent URL to the clipboard.
    Copy(String),
    /// Give up the browser sign-in for this address.
    CancelOAuth(String),
}

/// The first run, as far as it has got.
pub struct FirstRun {
    status: Status,
    address: Input,
    name: Input,
    password: Input,
    incoming: Input,
    outgoing: Input,
    client_id: Input,
    client_secret: Input,
    field: Field,
    /// Where a browser sign-in waits for the person.
    sign_in: Option<postio_ui::onboarding::BrowserSignIn>,
    /// The servers, once found or typed.
    settings: Option<Settings>,
    /// Signing in again to an account that exists, not adding one.
    repair: bool,
    /// Asked for from the mail, with an account already there to go back to.
    another: bool,
}

/// Without the password, which nothing prints.
impl std::fmt::Debug for FirstRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FirstRun")
            .field("status", &self.status)
            .field("address", &self.address.value())
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl Default for FirstRun {
    fn default() -> Self {
        FirstRun {
            status: Status::Idle,
            address: Input::default(),
            name: Input::default(),
            password: Input::default(),
            incoming: Input::default(),
            outgoing: Input::default(),
            client_id: Input::default(),
            client_secret: Input::default(),
            field: Field::Address,
            sign_in: None,
            settings: None,
            repair: false,
            another: false,
        }
    }
}

impl FirstRun {
    /// Signing in again to `account`, whose password the keyring no longer
    /// has or has wrong: its servers as saved, the password to type.
    pub fn repair(account: &postio_model::Account) -> Self {
        let settings = postio_ui::onboarding::configured(account);
        FirstRun {
            status: Status::Reauthenticate(settings.clone()),
            address: Input::default().with_value(account.address.address.clone()),
            name: Input::default().with_value(account.display_name.clone()),
            field: Field::Password,
            settings: Some(settings),
            repair: true,
            ..FirstRun::default()
        }
    }

    /// Adding an account beside the ones there are.
    pub fn another() -> Self {
        FirstRun {
            another: true,
            ..FirstRun::default()
        }
    }

    /// Whether there is mail to go back to: this was asked for, not the
    /// first screen of a store with no account.
    pub fn leavable(&self) -> bool {
        self.repair || self.another
    }

    /// What the screen is for.
    pub fn heading(&self) -> &'static str {
        if self.repair {
            "Sign in again"
        } else if self.another {
            "Add an account"
        } else {
            "Add your first account"
        }
    }

    /// The keys, while nothing more pressing is to be said.
    pub fn hint(&self) -> &'static str {
        if self.leavable() {
            "Enter moves on. Tab changes field. Esc goes back."
        } else {
            "Enter moves on. Tab changes field. Ctrl+Q quits."
        }
    }

    /// Whether this is a sign-in again rather than a first account.
    pub fn repairing(&self) -> bool {
        self.repair
    }

    /// Where the first run is.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Where the keyboard is.
    pub fn field(&self) -> Field {
        self.field
    }

    /// The servers found, or suggested for typing.
    pub fn settings(&self) -> Option<&Settings> {
        self.settings.as_ref()
    }

    /// Whether the servers have to be typed: nothing was found for the
    /// address, as the desktop's manual step.
    pub fn manual(&self) -> bool {
        matches!(self.status, Status::Manual { .. })
            || (matches!(self.status, Status::Failed(_)) && self.settings.is_none())
    }

    /// What a field holds, for drawing. The password as dots.
    pub fn value(&self, field: Field) -> String {
        match field {
            Field::Address => self.address.value().to_owned(),
            Field::Name => self.name.value().to_owned(),
            Field::Password => "•".repeat(self.password.value().chars().count()),
            Field::Incoming => self.incoming.value().to_owned(),
            Field::Outgoing => self.outgoing.value().to_owned(),
            Field::ClientId => self.client_id.value().to_owned(),
            Field::ClientSecret => "•".repeat(self.client_secret.value().chars().count()),
        }
    }

    /// Whether this provider signs in in a browser rather than with a
    /// password.
    pub fn browser(&self) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|settings| settings.oauth_sign_in)
            && !self.manual()
    }

    /// Where the browser sign-in waits, while it does.
    pub fn sign_in(&self) -> Option<&postio_ui::onboarding::BrowserSignIn> {
        self.sign_in.as_ref()
    }

    /// The consent URL came back: the sign-in waits for the person.
    pub fn consent(&mut self, sign_in: postio_ui::onboarding::BrowserSignIn) {
        self.sign_in = Some(sign_in);
        self.status = Status::WaitingForBrowser;
    }

    /// The address being signed in.
    pub fn address(&self) -> String {
        self.address.value().trim().to_owned()
    }

    /// The fields shown at this step, in order.
    pub fn fields(&self) -> Vec<Field> {
        match (&self.status, self.manual()) {
            (Status::Idle | Status::Probing, _) => vec![Field::Address],
            (_, true) => vec![
                Field::Address,
                Field::Name,
                Field::Incoming,
                Field::Outgoing,
                Field::Password,
            ],
            _ if self.browser() => vec![
                Field::Address,
                Field::Name,
                Field::ClientId,
                Field::ClientSecret,
            ],
            _ => vec![Field::Address, Field::Name, Field::Password],
        }
    }

    /// What discovery found.
    pub fn discovered(&mut self, status: Status) {
        match &status {
            Status::Found(settings) => {
                self.settings = Some(settings.clone());
                self.field = if settings.oauth_sign_in {
                    Field::ClientId
                } else {
                    Field::Password
                };
            }
            Status::Manual { suggestion } => {
                self.settings = suggestion.clone();
                let line = |server: &Server| {
                    if server.host.is_empty() {
                        String::new()
                    } else {
                        format!("{}:{}", server.host, server.port)
                    }
                };
                if let Some(settings) = suggestion {
                    self.incoming = Input::default().with_value(line(&settings.imap));
                    self.outgoing = Input::default().with_value(line(&settings.smtp));
                }
                self.field = Field::Incoming;
            }
            _ => {}
        }
        self.status = status;
    }

    /// The host could not be asked, or said no.
    pub fn failed(&mut self, sentence: String) {
        self.sign_in = None;
        self.field = if self.browser() {
            Field::ClientId
        } else {
            Field::Password
        };
        self.status = Status::Failed(sentence);
    }

    /// The account is saved: the last question is how far back to sync.
    pub fn added(&mut self) {
        self.status = Status::SyncWindow;
    }

    /// A key, and what it asks for.
    pub fn key(&mut self, key: KeyEvent) -> Asked {
        if self.status == Status::SyncWindow {
            return match key.code {
                KeyCode::Char(digit @ '1'..='3') => {
                    Asked::Start(SyncWindow::ALL[usize::from(digit as u8 - b'1')])
                }
                _ => Asked::Nothing,
            };
        }
        if self.status == Status::WaitingForBrowser {
            // Nothing leaves this machine on its own: Enter opens the URL,
            // `y` copies it, Escape gives the sign-in up.
            let url = self
                .sign_in
                .as_ref()
                .map(|sign_in| sign_in.authorize_url.clone())
                .unwrap_or_default();
            return match key.code {
                KeyCode::Enter if !url.is_empty() => Asked::Open(url),
                KeyCode::Char('y') if !url.is_empty() => Asked::Copy(url),
                KeyCode::Esc => {
                    self.sign_in = None;
                    if let Some(settings) = self.settings.clone() {
                        self.status = Status::Found(settings);
                    }
                    Asked::CancelOAuth(self.address())
                }
                _ => Asked::Nothing,
            };
        }
        if self.status.is_busy() {
            return Asked::Nothing;
        }
        let shown = self.fields();
        let at = shown
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        match key.code {
            KeyCode::Tab => {
                self.field = shown[(at + 1) % shown.len()];
                Asked::Nothing
            }
            KeyCode::BackTab => {
                self.field = shown[(at + shown.len() - 1) % shown.len()];
                Asked::Nothing
            }
            KeyCode::Enter if self.field == Field::Address => {
                let address = self.address.value().trim().to_owned();
                if !postio_ui::onboarding::looks_like_an_address(&address) {
                    return Asked::Nothing;
                }
                self.status = Status::Probing;
                Asked::Discover(address)
            }
            KeyCode::Enter => match self.submission() {
                Some(submission) if submission.oauth_client.is_some() => {
                    self.status = Status::Connecting;
                    Asked::BeginOAuth(Box::new(submission))
                }
                Some(submission) => {
                    self.status = Status::Connecting;
                    Asked::Add(Box::new(submission))
                }
                None => {
                    self.field = shown[(at + 1) % shown.len()];
                    Asked::Nothing
                }
            },
            _ => {
                let event = crossterm::event::Event::Key(key);
                let input = match self.field {
                    Field::Address => &mut self.address,
                    Field::Name => &mut self.name,
                    Field::Password => &mut self.password,
                    Field::Incoming => &mut self.incoming,
                    Field::Outgoing => &mut self.outgoing,
                    Field::ClientId => &mut self.client_id,
                    Field::ClientSecret => &mut self.client_secret,
                };
                input.handle_event(&event);
                Asked::Nothing
            }
        }
    }

    /// What would be submitted, once there is enough of it.
    fn submission(&self) -> Option<Submission> {
        let address = self.address.value().trim().to_owned();
        if self.browser() {
            let client_id = self.client_id.value().trim().to_owned();
            if address.is_empty() || client_id.is_empty() {
                return None;
            }
            let secret = self.client_secret.value().trim();
            return Some(Submission {
                address,
                name: self.name.value().trim().to_owned(),
                password: String::new(),
                settings: self.settings.clone().unwrap_or_default(),
                oauth_client: Some(postio_ui::onboarding::OAuthClientSubmission {
                    client_id,
                    client_secret: (!secret.is_empty()).then(|| secret.to_owned()),
                }),
            });
        }
        if address.is_empty() || self.password.value().is_empty() {
            return None;
        }
        let mut settings = self.settings.clone().unwrap_or_default();
        if self.manual() {
            settings.imap = typed_server(self.incoming.value(), 993)?;
            settings.smtp = typed_server(self.outgoing.value(), 465)?;
            if settings.login.is_empty() {
                settings.login = address.clone();
            }
            settings.source = "entered by hand".to_owned();
        }
        Some(Submission {
            address,
            name: self.name.value().trim().to_owned(),
            password: self.password.value().to_owned(),
            settings,
            oauth_client: None,
        })
    }
}

/// `host` or `host:port` as a server, over TLS unless the port is one that
/// starts plain (143, 587, 25).
fn typed_server(typed: &str, default_port: u16) -> Option<Server> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let (host, port) = match typed.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (typed, default_port),
    };
    let security = match port {
        143 | 587 | 25 => postio_model::TransportSecurity::StartTls,
        _ => postio_model::TransportSecurity::Tls,
    };
    Some(Server {
        host: host.to_owned(),
        port,
        security,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heading_and_the_way_out_say_which_run_this_is() {
        let first = FirstRun::default();
        assert_eq!(first.heading(), "Add your first account");
        assert!(!first.hint().contains("Esc"), "nothing to go back to");

        let another = FirstRun::another();
        assert_eq!(another.heading(), "Add an account");
        assert!(
            another.hint().contains("Esc goes back"),
            "{}",
            another.hint()
        );
    }
}
