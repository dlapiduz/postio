//! The known-sender popover at the boundary (specs/011-focus-contacts S6,
//! handoff screen 04).
//!
//! The controller decides what it says and shapes it (`postio_ui::
//! contact_popover`, `postio_focus`); the Mac draws `FocusContactPopover`,
//! whole, and reports what the person did that is not a key: a click on a
//! sender's name, a click on a segment of the arrival control. The keys
//! (`i`, the arrows, `0`, Return, `c`, Esc) are commands `key` resolves in
//! the popover's context and `invoke` runs.

use crate::session::Session;

/// Where a sender's mail arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactArrivalFfi {
    /// In the inbox, and never filtered away.
    Inbox,
    /// Held for their digest.
    Digest,
    /// Filed away as Filtered.
    Filtered,
}

impl From<ContactArrivalFfi> for postio_model::ContactArrival {
    fn from(arrival: ContactArrivalFfi) -> Self {
        match arrival {
            ContactArrivalFfi::Inbox => Self::Inbox,
            ContactArrivalFfi::Digest => Self::Digest,
            ContactArrivalFfi::Filtered => Self::Filtered,
        }
    }
}

impl From<postio_model::ContactArrival> for ContactArrivalFfi {
    fn from(arrival: postio_model::ContactArrival) -> Self {
        match arrival {
            postio_model::ContactArrival::Inbox => Self::Inbox,
            postio_model::ContactArrival::Digest => Self::Digest,
            postio_model::ContactArrival::Filtered => Self::Filtered,
        }
    }
}

/// What an avatar draws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AvatarFfi {
    /// Up to two letters, or the kind's glyph.
    pub text: String,
    /// The fill, `0xRRGGBB`.
    pub rgb: u32,
    /// A circle; otherwise a rounded square.
    pub round: bool,
}

/// One of the three figures under the name.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverTileFfi {
    /// "431", "today", "—".
    pub value: String,
    /// "messages".
    pub caption: String,
}

/// One thing open between the user and the person.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OpenLineFfi {
    /// "Waiting on you", "Waiting on Ada".
    pub label: String,
    /// What it is.
    pub text: String,
    /// Whether the move is the user's: the label is drawn in the accent.
    pub yours: bool,
    /// The message it hangs on.
    pub message: i64,
}

/// One segment of the arrival control.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ArrivalOptionFfi {
    /// "Inbox", "Digest", "Filtered out".
    pub label: String,
    /// What a click on it says to `focus_contact_arrival`.
    pub arrival: ContactArrivalFfi,
    /// Whether it is the one in force.
    pub on: bool,
}

/// One of the buttons along the foot.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverActionFfi {
    /// "All mail", "Write", "Open card".
    pub label: String,
    /// The command a click runs, by its stable id, through `invoke`.
    pub command: String,
    /// Its key as the keymap in force spells it.
    pub key: Option<String>,
    /// Drawn filled: the ↩ action.
    pub primary: bool,
    /// Whether it can be used now.
    pub enabled: bool,
}

/// The known-sender popover, drawn whole.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactPopoverViewFfi {
    /// The address it is about.
    pub address: String,
    /// The avatar.
    pub avatar: AvatarFfi,
    /// The name.
    pub name: String,
    /// The address under it.
    pub subline: String,
    /// "Finance lead, Northwind Partners".
    pub role: String,
    /// Three tiles.
    pub tiles: Vec<PopoverTileFfi>,
    /// Up to two open lines.
    pub open: Vec<OpenLineFfi>,
    /// "Mail from Ada arrives in".
    pub arrival_title: String,
    /// Inbox, Digest, Filtered out.
    pub arrival: Vec<ArrivalOptionFfi>,
    /// All mail, Write, Open card.
    pub actions: Vec<PopoverActionFfi>,
    /// The arrival control's own keys: "Left/Right" choose, "0" default
    /// rules.
    pub hints: Vec<crate::focus_surfaces::FocusHintFfi>,
}

impl From<postio_ui::contact_popover::PopoverView> for ContactPopoverViewFfi {
    fn from(view: postio_ui::contact_popover::PopoverView) -> Self {
        ContactPopoverViewFfi {
            address: view.address,
            avatar: AvatarFfi {
                text: view.avatar.text,
                rgb: view.avatar.rgb,
                round: view.avatar.round,
            },
            name: view.name,
            subline: view.subline,
            role: view.role,
            tiles: view
                .tiles
                .into_iter()
                .map(|tile| PopoverTileFfi {
                    value: tile.value,
                    caption: tile.caption,
                })
                .collect(),
            open: view
                .open
                .into_iter()
                .map(|line| OpenLineFfi {
                    label: line.label,
                    text: line.text,
                    yours: line.yours,
                    message: line.message.get(),
                })
                .collect(),
            arrival_title: view.arrival_title,
            arrival: view
                .arrival
                .into_iter()
                .map(|option| ArrivalOptionFfi {
                    label: option.label,
                    arrival: option.arrival.into(),
                    on: option.on,
                })
                .collect(),
            actions: view
                .actions
                .into_iter()
                .map(|action| PopoverActionFfi {
                    label: action.label,
                    command: action.command.as_str().to_owned(),
                    key: action.key,
                    primary: action.primary,
                    enabled: action.enabled,
                })
                .collect(),
            hints: view.hints.into_iter().map(Into::into).collect(),
        }
    }
}

#[uniffi::export]
impl Session {
    /// A click on a sender's name: the popover for this address, as `i`
    /// would draw it for the row. It arrives as `FocusContactPopover` once
    /// the host has answered, and not at all for an address nobody owns.
    pub fn focus_contact_popover_at(&self, address: String) {
        self.focus_driver()
            .input(postio_focus::Input::ContactPopoverAt(address));
    }

    /// A click on a segment of the arrival control: where their mail
    /// arrives from now, applied at once, with the engine's undo toast.
    pub fn focus_contact_arrival(&self, arrival: ContactArrivalFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ContactArrival(arrival.into()));
    }
}
