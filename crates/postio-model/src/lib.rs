//! Postio's domain model: the types every other crate speaks. Pure: no SQL, IMAP, GTK or I/O, so a
//! second protocol or frontend needs no reshaping; CI enforces it. See `ids` for identifier rules
//! and `Flag` for flag invariants. The `test-corpus` feature ships the `.eml` fixture corpus and
//! its loader; it is dev-dependency only.

pub mod account;
pub mod address;
pub mod attachment;
pub mod card;
pub mod clock;
pub mod contact;
pub mod contact_form;
pub mod contact_group;
pub mod contact_settings;
pub mod draft;
pub mod egress;
pub mod fetch;
pub mod filing;
pub mod flag;
pub mod fold;
pub mod headers;
pub mod ids;
pub mod label;
pub mod listing;
pub mod mailbox;
pub mod mailto;
pub mod mention;
pub mod message;
pub mod mime;
pub mod net;
pub mod operation;
pub mod outgoing;
pub mod promoted;
pub mod reply;
pub mod scope;
pub mod signature;
pub mod signature_default;
pub mod size;
pub mod subject;
pub mod summary;
pub mod sweep;
pub mod sync;
#[cfg(feature = "test-corpus")]
pub mod test_corpus;
pub mod thread;
pub mod thread_view;
pub mod threading;
pub mod unsubscribe;

pub use account::{
    Account, AccountScope, AuthMethod, Backend, Identity, OAuthConfig, ServerConfig, Signature,
    TransportSecurity,
};
pub use address::EmailAddress;
pub use attachment::{Attachment, Disposition};
pub use contact::{
    AddressMove, CARD_RECENT, Contact, ContactAddress, ContactCardRead, ContactDetail,
    ContactGlance, ContactKind, ContactListRow, ContactSource, ContactState, ContactStats,
    ContactTab, ContactView, ContactsOverview, ContactsPageRead, FileChip, IssueRead, JoinReceipt,
    JoinSuggestion, KindSignals, KindSource, MergeGroup, OpenItem, OpenSide, PersonEdit,
    PersonFields, RECENT, REVIEW_CAP, RecentMessage, ReviewSummary, ShowAs, SuggestionReason,
    month_key,
};
pub use contact_form::{
    AddressLabel, AddressNote, ContactForm, DetailKind, EvidenceKind, FormAddress, FormProblem,
    Introduction, KeptDetail, PairEvidence, SeenItem, SenderPrior, SignatureDetails, split_name,
};
pub use contact_group::ContactGroup;
pub use contact_settings::{
    CardSwitch, ContactArrival, ContactSettings, ContactSuggestions, DigestSetting, StoppedMarker,
    Suggestion,
};
pub use draft::{Draft, DraftKind, DraftState};
pub use flag::{Flag, FlagSet};
pub use headers::{Header, Headers};
pub use ids::{
    AccountId, AddressId, AttachmentId, BlobId, ContactGroupId, ContactId, DeliveryId, DraftId,
    Generation, IdentityId, LabelId, MailboxId, MessageId, ModSeq, OperationId, ReminderId,
    RemoteId, RfcMessageId, SignatureId, ThreadId, Uid, UidValidity,
};
pub use label::Label;
pub use mailbox::{Mailbox, MailboxCounts, MailboxRole, RoleKind, RoleOverrides};
pub use message::{
    BodyState, ContentIdentity, LocalSyncState, Message, MessageBody, ServerIdentifiers,
};
pub use mime::{ParsedMessage, ParsedPart};
pub use operation::{Operation, OperationRange, OperationState, OperationTarget};
pub use outgoing::{BuiltMessage, CalendarMethod, CalendarPart, OutgoingAttachment};
pub use scope::{Arrival, FocusScope, ListScope, Reaction};
pub use subject::{is_reply, normalize_subject};
pub use sweep::{ChangeFeed, FlagPass, SweepPolicy};
pub use sync::{FullResyncReason, MailboxStatus, ResyncPlan, SyncState};
pub use thread::Thread;
pub use thread_view::{ForwardForm, Forwarded, Line, ThreadDraft, ThreadMember, ThreadView};
pub use threading::{
    Assignment, ThreadCue, ThreadIndex, ThreadNode, assign, claimed_ids, descendants,
};
pub use unsubscribe::UnsubscribeActivation;
