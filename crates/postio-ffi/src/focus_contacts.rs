//! The known-sender popover and the Contacts view at the boundary (specs/011-focus-contacts S6,
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

/// How a sender not yet saved is introduced (handoff screen 05).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverIntroFfi {
    /// "Sent 1 message, today 13:47".
    pub sent: String,
    /// "Not saved yet. First email today; Ben Adeyemi is on copy."
    pub status: String,
    /// "From their signature"; none when the signature said nothing.
    pub signature_title: Option<String>,
    /// The signature's lines.
    pub signature_lines: Vec<String>,
    /// "Read on this Mac. Added only if you save."
    pub signature_foot: Option<String>,
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

/// A kind tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactTabFfi {
    /// Everyone.
    All,
    /// People.
    People,
    /// Companies.
    Companies,
    /// Lists and newsletters.
    Lists,
    /// Automated senders.
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

/// One tab in the strip.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactTabViewFfi {
    /// Which; what a click says to `focus_contacts_tab`.
    pub tab: ContactTabFfi,
    /// "People".
    pub title: String,
    /// "612"; `None` for All.
    pub count: Option<String>,
    /// Whether it is chosen.
    pub on: bool,
}

/// The "Worth a look" line.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactReviewLineFfi {
    /// "Worth a look:".
    pub lead: String,
    /// What waits.
    pub text: String,
    /// "Review".
    pub button: String,
    /// Its key.
    pub key: Option<String>,
    /// Whether it does anything yet.
    pub enabled: bool,
}

/// The strip over the list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsStripFfi {
    /// "Contacts".
    pub title: String,
    /// `g c`.
    pub key: Option<String>,
    /// "1,533".
    pub total: String,
    /// The five tabs.
    pub tabs: Vec<ContactTabViewFfi>,
    /// "Filter contacts".
    pub filter_placeholder: String,
    /// The filter's key.
    pub filter_key: Option<String>,
    /// "New contact".
    pub new_label: String,
    /// Its key.
    pub new_key: Option<String>,
    /// Whether it does anything yet.
    pub new_enabled: bool,
    /// The review line; hidden when `None`.
    pub review: Option<ContactReviewLineFfi>,
}

/// The footer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsFooterFfi {
    /// The keys that work here.
    pub hints: Vec<crate::focus_surfaces::FocusHintFfi>,
    /// The counts and the catch-up.
    pub status: String,
}

/// The Contacts view's frame, drawn whole.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsViewFfi {
    /// The strip.
    pub strip: ContactsStripFfi,
    /// The footer.
    pub footer: ContactsFooterFfi,
    /// How many rows the list has.
    pub total: u32,
    /// What the filter holds.
    pub filter: String,
    /// Whether the counts have been read.
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

/// One row of the Contacts list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactRowViewFfi {
    /// The person.
    pub id: i64,
    /// The avatar.
    pub avatar: AvatarFfi,
    /// The name.
    pub name: String,
    /// The tag pill.
    pub tag: Option<String>,
    /// The address, with `+2`.
    pub address: String,
    /// When they were last in touch.
    pub when: String,
    /// The count line.
    pub count_line: String,
    /// The letter heading above this row, when it starts one.
    pub letter: Option<String>,
    /// Whether the person is marked for a merge.
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

/// A button along the card's head.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardButtonFfi {
    /// "Write".
    pub label: String,
    /// The command, by its stable id.
    pub command: String,
    /// Its key.
    pub key: Option<String>,
    /// The filled one.
    pub primary: bool,
    /// Whether it does anything yet.
    pub enabled: bool,
}

/// One address on the card.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardAddressFfi {
    /// The address.
    pub address: String,
    /// The star.
    pub primary: bool,
    /// Its note.
    pub note: String,
    /// "bounced 2 Sep".
    pub bounced: Option<String>,
    /// Whether it can be split off into a person of its own.
    pub can_split: bool,
}

/// One switch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardSwitchFfi {
    /// Its title.
    pub title: String,
    /// The line under it.
    pub note: String,
    /// Whether it is on.
    pub on: bool,
    /// Whether a click turns it on or off.
    pub can_toggle: bool,
}

/// The twelve-month strip.
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
    /// Its label.
    pub label: String,
    /// What it is.
    pub text: String,
    /// "opened".
    pub state: String,
    /// Whether the move is the user's.
    pub yours: bool,
}

/// A file chip.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CardFileFfi {
    /// `XLSX`.
    pub kind: String,
    /// Its name.
    pub name: String,
}

/// A card, drawn whole.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactCardViewFfi {
    /// The person.
    pub id: i64,
    /// The avatar, at 64.
    pub avatar: AvatarFfi,
    /// The name.
    pub name: String,
    /// The line under it.
    pub role: String,
    /// The head's buttons.
    pub buttons: Vec<CardButtonFfi>,
    /// "Addresses".
    pub addresses_title: String,
    /// The addresses.
    pub addresses: Vec<CardAddressFfi>,
    /// "Also seen as ...".
    pub also_seen: Option<String>,
    /// "How mail from Ada arrives".
    pub arrival_title: String,
    /// Inbox, Digest, Filtered out.
    pub arrival: Vec<ArrivalOptionFfi>,
    /// The switches.
    pub switches: Vec<CardSwitchFfi>,
    /// "Notes".
    pub notes_title: String,
    /// The note.
    pub notes: Option<String>,
    /// "Kept on this Mac."
    pub notes_foot: String,
    /// "Your mail together".
    pub stats_title: String,
    /// Four tiles.
    pub tiles: Vec<PopoverTileFfi>,
    /// The strip.
    pub months: CardMonthsFfi,
    /// "Open between you".
    pub lines_title: String,
    /// The lines.
    pub lines: Vec<CardLineFfi>,
    /// "Recent files from Ada".
    pub files_title: Option<String>,
    /// The chips.
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
    /// The Contacts list's row at `index`, or `None` while its page is on its
    /// way: a miss asks for the page, and `FocusContactsRows` says when to
    /// draw it. Synchronous; what the table calls for every visible row.
    pub fn focus_contacts_row(&self, index: u32) -> Option<ContactRowViewFfi> {
        self.focus_driver().contacts_row(index).map(Into::into)
    }

    /// How many rows the Contacts list has.
    pub fn focus_contacts_total(&self) -> u32 {
        self.focus_driver().contacts_total()
    }

    /// Where the Contacts cursor is.
    pub fn focus_contacts_cursor(&self) -> Option<u32> {
        self.focus_driver().contacts_cursor()
    }

    /// The card beside the list, once it has been read.
    pub fn focus_contacts_card(&self) -> Option<ContactCardViewFfi> {
        self.focus_driver().contacts_card().map(Into::into)
    }

    /// A click on a kind tab.
    pub fn focus_contacts_tab(&self, tab: ContactTabFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsTab(tab.into()));
    }

    /// The filter field holds `text` now.
    pub fn focus_contacts_filter(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsFilterTyped(text));
    }

    /// A click on the Contacts row at `index`.
    pub fn focus_contacts_point(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsPoint(index));
    }
}
