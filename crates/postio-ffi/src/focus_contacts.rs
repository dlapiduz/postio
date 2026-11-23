//! The known-sender popover and the Contacts view. The Mac draws `FocusContactPopover` whole and
//! reports what is not a key (a click on a sender's name or an arrival segment); the keys are
//! commands `key` resolves and `invoke` runs.

use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactArrivalFfi {
    Inbox,
    Digest,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AvatarFfi {
    /// Up to two letters, or the kind's glyph.
    pub text: String,
    /// The fill, `0xRRGGBB`.
    pub rgb: u32,
    /// A circle; otherwise a rounded square.
    pub round: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverTileFfi {
    /// "431", "today", "—".
    pub value: String,
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OpenLineFfi {
    /// "Waiting on you", "Waiting on Ada".
    pub label: String,
    pub text: String,
    /// Whether the move is the user's: the label is drawn in the accent.
    pub yours: bool,
    pub message: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ArrivalOptionFfi {
    /// "Inbox", "Digest", "Filtered out".
    pub label: String,
    pub arrival: ContactArrivalFfi,
    pub on: bool,
}

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
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverIntroFfi {
    /// "Sent 1 message, today 13:47".
    pub sent: String,
    /// "Not saved yet. First email today; Ben Adeyemi is on copy."
    pub status: String,
    /// "From their signature"; none when the signature said nothing.
    pub signature_title: Option<String>,
    pub signature_lines: Vec<String>,
    /// "Read on this Mac. Added only if you save."
    pub signature_foot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactPopoverViewFfi {
    pub address: String,
    pub avatar: AvatarFfi,
    pub name: String,
    pub subline: String,
    /// "Finance lead, Northwind Partners".
    pub role: String,
    pub tiles: Vec<PopoverTileFfi>,
    pub open: Vec<OpenLineFfi>,
    /// "Mail from Ada arrives in".
    pub arrival_title: String,
    pub arrival: Vec<ArrivalOptionFfi>,
    /// All mail, Write, Open card; or Add contact, Not now, Ignore sender.
    pub actions: Vec<PopoverActionFfi>,
    /// For a sender not yet saved: in place of the tiles and open lines.
    pub intro: Option<PopoverIntroFfi>,
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
            intro: view.intro.map(|intro| PopoverIntroFfi {
                sent: intro.sent,
                status: intro.status,
                signature_title: intro.signature_title,
                signature_lines: intro.signature_lines,
                signature_foot: intro.signature_foot,
            }),
            hints: view.hints.into_iter().map(Into::into).collect(),
        }
    }
}

#[uniffi::export]
impl Session {
    /// A click on a sender's name: the popover for this address, arriving as `FocusContactPopover`;
    /// none for an address nobody owns.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactTabFfi {
    All,
    People,
    Companies,
    Lists,
    Automated,
}

impl From<ContactTabFfi> for postio_model::ContactTab {
    fn from(tab: ContactTabFfi) -> Self {
        match tab {
            ContactTabFfi::All => Self::All,
            ContactTabFfi::People => Self::People,
            ContactTabFfi::Companies => Self::Companies,
            ContactTabFfi::Lists => Self::Lists,
            ContactTabFfi::Automated => Self::Automated,
        }
    }
}

impl From<postio_model::ContactTab> for ContactTabFfi {
    fn from(tab: postio_model::ContactTab) -> Self {
        match tab {
            postio_model::ContactTab::All => Self::All,
            postio_model::ContactTab::People => Self::People,
            postio_model::ContactTab::Companies => Self::Companies,
            postio_model::ContactTab::Lists => Self::Lists,
            postio_model::ContactTab::Automated => Self::Automated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactTabViewFfi {
    /// Which; what a click says to `focus_contacts_tab`.
    pub tab: ContactTabFfi,
    /// "People".
    pub title: String,
    /// "612"; `None` for All.
    pub count: Option<String>,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactReviewLineFfi {
    pub lead: String,
    pub text: String,
    pub button: String,
    pub key: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsStripFfi {
    pub title: String,
    /// `g c`.
    pub key: Option<String>,
    /// "1,533".
    pub total: String,
    pub tabs: Vec<ContactTabViewFfi>,
    /// "Filter contacts".
    pub filter_placeholder: String,
    pub filter_key: Option<String>,
    /// "New contact".
    pub new_label: String,
    pub new_key: Option<String>,
    pub new_enabled: bool,
    /// The review line; hidden when `None`.
    pub review: Option<ContactReviewLineFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsFooterFfi {
    pub hints: Vec<crate::focus_surfaces::FocusHintFfi>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsViewFfi {
    pub strip: ContactsStripFfi,
    pub footer: ContactsFooterFfi,
    pub total: u32,
    pub filter: String,
    pub loaded: bool,
}

impl From<postio_focus::ContactsView> for ContactsViewFfi {
    fn from(view: postio_focus::ContactsView) -> Self {
        ContactsViewFfi {
            strip: ContactsStripFfi {
                title: view.strip.title,
                key: view.strip.key,
                total: view.strip.total,
                tabs: view
                    .strip
                    .tabs
                    .into_iter()
                    .map(|tab| ContactTabViewFfi {
                        tab: tab.tab.into(),
                        title: tab.title,
                        count: tab.count,
                        on: tab.on,
                    })
                    .collect(),
                filter_placeholder: view.strip.filter_placeholder,
                filter_key: view.strip.filter_key,
                new_label: view.strip.new_label,
                new_key: view.strip.new_key,
                new_enabled: view.strip.new_enabled,
                review: view.strip.review.map(|line| ContactReviewLineFfi {
                    lead: line.lead,
                    text: line.text,
                    button: line.button,
                    key: line.key,
                    enabled: line.enabled,
                }),
            },
            footer: ContactsFooterFfi {
                hints: view.footer.hints.into_iter().map(Into::into).collect(),
                status: view.footer.status,
            },
            total: view.total,
            filter: view.filter,
            loaded: view.loaded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactRowViewFfi {
    pub id: i64,
    pub avatar: AvatarFfi,
    pub name: String,
    pub tag: Option<String>,
    /// The address, with `+2`.
    pub address: String,
    pub when: String,
    pub count_line: String,
    pub letter: Option<String>,
    pub marked: bool,
}

impl From<postio_ui::contacts_view::RowView> for ContactRowViewFfi {
    fn from(row: postio_ui::contacts_view::RowView) -> Self {
        ContactRowViewFfi {
            id: row.id.get(),
            avatar: row.avatar.into(),
            name: row.name,
            tag: row.tag,
            address: row.address,
            when: row.when,
            count_line: row.count_line,
            letter: row.letter.map(|letter| letter.to_string()),
            marked: row.marked,
        }
    }
}

impl From<postio_ui::contact_popover::Avatar> for AvatarFfi {
    fn from(avatar: postio_ui::contact_popover::Avatar) -> Self {
        AvatarFfi {
            text: avatar.text,
            rgb: avatar.rgb,
            round: avatar.round,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardButtonFfi {
    /// "Write".
    pub label: String,
    pub command: String,
    pub key: Option<String>,
    pub primary: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardAddressFfi {
    pub address: String,
    pub primary: bool,
    pub note: String,
    /// "bounced 2 Sep".
    pub bounced: Option<String>,
    /// Whether it can be split off into a person of its own.
    pub can_split: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardSwitchFfi {
    pub title: String,
    pub note: String,
    pub on: bool,
    pub can_toggle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardMonthsFfi {
    /// Twelve heights, oldest first, in thousandths of the tallest month
    /// (0 to 1000).
    pub bars: Vec<u16>,
    /// "Oct 2025".
    pub from: String,
    /// "messages per month".
    pub caption: String,
    /// "Sep 2026".
    pub to: String,
}

/// A line of "Open between you" or "Recent issues".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardLineFfi {
    pub label: String,
    pub text: String,
    /// "opened".
    pub state: String,
    pub yours: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardFileFfi {
    /// `XLSX`.
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactCardViewFfi {
    pub id: i64,
    pub avatar: AvatarFfi,
    pub name: String,
    pub role: String,
    pub buttons: Vec<CardButtonFfi>,
    pub addresses_title: String,
    pub addresses: Vec<CardAddressFfi>,
    /// "Also seen as ...".
    pub also_seen: Option<String>,
    /// "How mail from Ada arrives".
    pub arrival_title: String,
    pub arrival: Vec<ArrivalOptionFfi>,
    pub switches: Vec<CardSwitchFfi>,
    pub notes_title: String,
    pub notes: Option<String>,
    pub notes_foot: String,
    pub stats_title: String,
    pub tiles: Vec<PopoverTileFfi>,
    pub months: CardMonthsFfi,
    pub lines_title: String,
    pub lines: Vec<CardLineFfi>,
    pub files_title: Option<String>,
    pub files: Vec<CardFileFfi>,
}

impl From<postio_ui::contacts_view::CardView> for ContactCardViewFfi {
    fn from(card: postio_ui::contacts_view::CardView) -> Self {
        ContactCardViewFfi {
            id: card.id.get(),
            avatar: card.avatar.into(),
            name: card.name,
            role: card.role,
            buttons: card
                .buttons
                .into_iter()
                .map(|button| CardButtonFfi {
                    label: button.label,
                    command: button.command.as_str().to_owned(),
                    key: button.key,
                    primary: button.primary,
                    enabled: button.enabled,
                })
                .collect(),
            addresses_title: card.addresses_title,
            addresses: card
                .addresses
                .into_iter()
                .map(|line| CardAddressFfi {
                    address: line.address,
                    primary: line.primary,
                    note: line.note,
                    bounced: line.bounced,
                    can_split: line.can_split,
                })
                .collect(),
            also_seen: card.also_seen,
            arrival_title: card.arrival_title,
            arrival: card
                .arrival
                .into_iter()
                .map(|option| ArrivalOptionFfi {
                    label: option.label,
                    arrival: option.arrival.into(),
                    on: option.on,
                })
                .collect(),
            switches: card
                .switches
                .into_iter()
                .map(|line| CardSwitchFfi {
                    title: line.title,
                    note: line.note,
                    on: line.on,
                    can_toggle: line.switch.is_some(),
                })
                .collect(),
            notes_title: card.notes_title,
            notes: card.notes,
            notes_foot: card.notes_foot,
            stats_title: card.stats_title,
            tiles: card
                .tiles
                .into_iter()
                .map(|tile| PopoverTileFfi {
                    value: tile.value,
                    caption: tile.caption,
                })
                .collect(),
            months: CardMonthsFfi {
                bars: card
                    .months
                    .bars
                    .into_iter()
                    .map(|bar| (bar.clamp(0.0, 1.0) * 1000.0).round() as u16)
                    .collect(),
                from: card.months.from,
                caption: card.months.caption,
                to: card.months.to,
            },
            lines_title: card.lines_title,
            lines: card
                .lines
                .into_iter()
                .map(|line| CardLineFfi {
                    label: line.label,
                    text: line.text,
                    state: line.state,
                    yours: line.yours,
                })
                .collect(),
            files_title: card.files_title,
            files: card
                .files
                .into_iter()
                .map(|file| CardFileFfi {
                    kind: file.kind,
                    name: file.name,
                })
                .collect(),
        }
    }
}

#[uniffi::export]
impl Session {
    /// The Contacts list's row at `index`, or `None` while its page is on its way: a miss asks for
    /// the page, and `FocusContactsRows` says when to draw it. Synchronous.
    pub fn focus_contacts_row(&self, index: u32) -> Option<ContactRowViewFfi> {
        self.focus_driver().contacts_row(index).map(Into::into)
    }

    pub fn focus_contacts_total(&self) -> u32 {
        self.focus_driver().contacts_total()
    }

    pub fn focus_contacts_cursor(&self) -> Option<u32> {
        self.focus_driver().contacts_cursor()
    }

    /// The card beside the list, once it has been read.
    pub fn focus_contacts_card(&self) -> Option<ContactCardViewFfi> {
        self.focus_driver().contacts_card().map(Into::into)
    }

    pub fn focus_contacts_tab(&self, tab: ContactTabFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsTab(tab.into()));
    }

    pub fn focus_contacts_filter(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsFilterTyped(text));
    }

    pub fn focus_contacts_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsPoint(index));
    }
}
