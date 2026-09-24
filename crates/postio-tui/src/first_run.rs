//! The first run: an address, its servers and a password, then how far back
//! to sync (US7).
//!
//! The desktop's first-run screen, in a terminal: the same three steps, the
//! same states (`postio_ui::onboarding::Status`), the same sentences. What is
//! typed is kept here; finding the servers and proving the password are the
//! daemon's (`Req::Discover`, `Req::AddAccount`), which do it as the desktop
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
}

/// The first run, as far as it has got.
pub struct FirstRun {
    status: Status,
    address: Input,
    name: Input,
    password: Input,
    incoming: Input,
    outgoing: Input,
    field: Field,
    /// The servers, once found or typed.
    settings: Option<Settings>,
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
            field: Field::Address,
            settings: None,
        }
    }
}

impl FirstRun {
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
        }
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
            _ => vec![Field::Address, Field::Name, Field::Password],
        }
    }

    /// What discovery found.
    pub fn discovered(&mut self, status: Status) {
        match &status {
            Status::Found(settings) => {
                self.settings = Some(settings.clone());
                self.field = Field::Password;
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

    /// The daemon could not be asked, or said no.
    pub fn failed(&mut self, sentence: String) {
        self.status = Status::Failed(sentence);
        self.field = Field::Password;
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
                };
                input.handle_event(&event);
                Asked::Nothing
            }
        }
    }

    /// What would be submitted, once there is enough of it.
    fn submission(&self) -> Option<Submission> {
        let address = self.address.value().trim().to_owned();
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
