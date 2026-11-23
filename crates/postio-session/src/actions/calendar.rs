use chrono::{DateTime, Duration, Utc};
use postio_calendar::{Edits, Method, NewEvent, Scope};
use postio_core::Command;
use postio_core::dispatch::CommandError;
use postio_core::undo::UndoKind;
use postio_model::calendar::{EditScope, EventFields, EventTarget};
use postio_model::draft::{Draft, DraftKind};
use postio_model::ids::{CalendarId, EventChangeId, EventId};
use postio_model::{Account, Identity};
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, CalendarRepository, ChangeKind, DraftRepository, HeldEvent,
};

use super::{Actions, RSVP_WINDOW, store_failure};
use crate::calendar::{NewObject, store_object_on};

pub(super) struct Changed {
    pub kind: UndoKind,
    pub inverse: Vec<Command>,
    pub lasts: Option<std::time::Duration>,
}

pub(super) struct Save<'a> {
    pub event: Option<&'a EventTarget>,
    pub calendar: Option<CalendarId>,
    pub fields: Option<&'a EventFields>,
    pub scope: Option<EditScope>,
    pub send: bool,
}

fn refused(error: impl std::fmt::Display) -> CommandError {
    tracing::warn!(%error, "an event verb could not read or write its object");
    CommandError::rejected("Postio could not change that event")
}

fn revert(change: EventChangeId) -> Command {
    Command::RevertEventChange { change }
}

fn scope_of(
    target: &EventTarget,
    scope: Option<EditScope>,
    recurs: bool,
) -> Result<Scope, CommandError> {
    if !recurs {
        return Ok(Scope::All);
    }
    let instance = || {
        target
            .recurrence_id
            .clone()
            .ok_or_else(|| CommandError::rejected("Postio cannot tell which occurrence that is"))
    };
    match scope {
        None => Err(CommandError::rejected(
            "Choose this occurrence, this and following, or all of them",
        )),
        Some(EditScope::All) => Ok(Scope::All),
        Some(EditScope::This) => Ok(Scope::This(instance()?)),
        Some(EditScope::ThisAndFollowing) => Ok(Scope::ThisAndFollowing(instance()?)),
    }
}

async fn held(connection: &Connection, event: EventId) -> Result<HeldEvent, CommandError> {
    let held = CalendarRepository::new(connection)
        .held(event)
        .await
        .map_err(store_failure)?
        .ok_or_else(|| CommandError::rejected("That event is no longer here"))?;
    if !held.writable {
        return Err(CommandError::rejected("That calendar is read-only"));
    }
    Ok(held)
}

async fn put(
    connection: &Connection,
    calendar: CalendarId,
    ics: &[u8],
    like: Option<&HeldEvent>,
    now: DateTime<Utc>,
) -> Result<EventId, CommandError> {
    store_object_on(
        connection,
        NewObject {
            calendar,
            ics,
            href: like.and_then(|held| held.href.clone()),
            etag: like.and_then(|held| held.etag.clone()),
            my_partstat: like.and_then(|held| held.my_partstat.clone()),
            source_message: like.and_then(|held| held.source_message),
        },
        &chrono::Local,
        now,
    )
    .await
    .map_err(|error| CommandError::rejected(error.to_string()))
}

async fn forget(connection: &Connection, event: EventId) -> Result<(), CommandError> {
    CalendarRepository::new(connection)
        .delete_event(event)
        .await
        .map_err(store_failure)?;
    postio_index::events::forget_event(connection, event)
        .await
        .map_err(refused)
}

async fn record(
    connection: &Connection,
    event: Option<EventId>,
    calendar: CalendarId,
    kind: ChangeKind,
    base: Option<&HeldEvent>,
    now: DateTime<Utc>,
) -> Result<EventChangeId, CommandError> {
    CalendarRepository::new(connection)
        .record_change(
            event,
            calendar,
            kind,
            base.map(|held| (held.object.as_slice(), held.etag.as_deref())),
            now,
        )
        .await
        .map_err(store_failure)
}

async fn organiser(
    connection: &Connection,
    calendar: CalendarId,
) -> Result<Option<(Account, Identity)>, CommandError> {
    let owner = CalendarRepository::new(connection)
        .list()
        .await
        .map_err(store_failure)?
        .into_iter()
        .find(|known| known.id == calendar)
        .and_then(|known| known.account);
    let accounts = AccountRepository::new(connection)
        .list_enabled()
        .await
        .map_err(store_failure)?;
    let account = match owner {
        Some(owner) => accounts.into_iter().find(|account| account.id == owner),
        None => accounts.into_iter().next(),
    };
    Ok(account.and_then(|account| {
        let identity = account.default_identity()?.clone();
        Some((account, identity))
    }))
}

fn edits(target: &EventTarget, fields: &EventFields) -> Edits {
    let shift = fields.start - target.occurrence.start;
    Edits {
        title: Some(fields.title.clone()),
        location: Some(fields.location.clone()),
        notes: Some(fields.notes.clone()),
        start: None,
        end: None,
        shift: (shift != Duration::zero()).then_some(shift),
        length: Some(fields.end - fields.start),
    }
}

impl Actions {
    pub(super) async fn save_event(&self, save: Save<'_>) -> Result<Changed, CommandError> {
        let fields = save
            .fields
            .ok_or_else(|| CommandError::rejected("Fill in the event first"))?;
        if fields.end < fields.start {
            return Err(CommandError::rejected(
                "An event cannot end before it starts",
            ));
        }
        if fields.title.trim().is_empty() {
            return Err(CommandError::rejected("Give the event a title"));
        }
        if save.send && fields.attendees.is_empty() {
            return Err(CommandError::rejected("Add someone to invite first"));
        }
        let now = super::now();
        let (connection, _permit) = self.connect().await?;
        let (event, calendar, mut inverse, kind) = match save.event {
            None => {
                let calendar = match save.calendar {
                    Some(calendar) => calendar,
                    None => {
                        CalendarRepository::new(&connection)
                            .local()
                            .await
                            .map_err(store_failure)?
                            .id
                    }
                };
                let from = organiser(&connection, calendar).await?;
                let ics = postio_calendar::new_object(
                    &NewEvent {
                        uid: postio_model::outgoing::unique_at("postio.invalid"),
                        title: fields.title.clone(),
                        start: fields.start,
                        end: fields.end,
                        all_day: fields.all_day,
                        location: fields.location.clone(),
                        notes: fields.notes.clone(),
                        organiser: from.map(|(_, identity)| identity.address),
                        attendees: fields.attendees.clone(),
                        alarm_minutes: fields.minutes_before,
                    },
                    now,
                    &chrono::Local,
                );
                let event = put(&connection, calendar, &ics, None, now).await?;
                let change = record(
                    &connection,
                    Some(event),
                    calendar,
                    ChangeKind::Create,
                    None,
                    now,
                )
                .await?;
                (event, calendar, vec![revert(change)], UndoKind::AddEvent)
            }
            Some(target) => {
                let before = held(&connection, target.occurrence.event).await?;
                let scope = scope_of(target, save.scope, before.recurs)?;
                let edited = postio_calendar::patch(
                    &before.object,
                    &edits(target, fields),
                    &scope,
                    &chrono::Local,
                )
                .map_err(refused)?;
                let inverse = self
                    .replace(
                        &connection,
                        &before,
                        &edited.object,
                        edited.split.as_deref(),
                        now,
                    )
                    .await?;
                (before.id, before.calendar, inverse, UndoKind::SaveEvent)
            }
        };
        tracing::info!(event = event.get(), "saved an event");
        if !save.send {
            return Ok(Changed {
                kind,
                inverse,
                lasts: None,
            });
        }
        let draft = self
            .invite(&connection, event, calendar, fields, now)
            .await?;
        inverse.push(Command::CancelSend { draft: Some(draft) });
        Ok(Changed {
            kind: UndoKind::InviteToEvent,
            inverse,
            lasts: Some(RSVP_WINDOW),
        })
    }

    async fn replace(
        &self,
        connection: &Connection,
        before: &HeldEvent,
        object: &[u8],
        split: Option<&[u8]>,
        now: DateTime<Utc>,
    ) -> Result<Vec<Command>, CommandError> {
        put(connection, before.calendar, object, Some(before), now).await?;
        let change = record(
            connection,
            Some(before.id),
            before.calendar,
            ChangeKind::Update,
            Some(before),
            now,
        )
        .await?;
        let mut inverse = vec![revert(change)];
        if let Some(split) = split {
            let added = put(connection, before.calendar, split, None, now).await?;
            let change = record(
                connection,
                Some(added),
                before.calendar,
                ChangeKind::Create,
                None,
                now,
            )
            .await?;
            inverse.push(revert(change));
        }
        Ok(inverse)
    }

    async fn invite(
        &self,
        connection: &Connection,
        event: EventId,
        calendar: CalendarId,
        fields: &EventFields,
        now: DateTime<Utc>,
    ) -> Result<postio_model::DraftId, CommandError> {
        let (account, identity) = organiser(connection, calendar)
            .await?
            .ok_or_else(|| CommandError::rejected("Add an account to send invitations from"))?;
        let object = CalendarRepository::new(connection)
            .held(event)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That event is no longer here"))?
            .object;
        let ics = postio_calendar::request(&object, Method::Request).map_err(refused)?;
        let mut draft = Draft::new(account.id);
        draft.use_identity(&identity);
        draft.kind = DraftKind::New;
        draft.to = fields.attendees.clone();
        draft.subject = format!("Invitation: {}", fields.title);
        draft.body.text = Some(invitation_text(fields, &identity));
        draft.calendar_reply = Some(String::from_utf8_lossy(&ics).into_owned());
        let until = now
            + Duration::from_std(RSVP_WINDOW).map_err(|_| {
                CommandError::rejected("That window is longer than Postio can hold")
            })?;
        DraftRepository::new(connection)
            .queue_send_at(&mut draft, now, until)
            .await
            .map_err(store_failure)?;
        Ok(draft.id)
    }

    pub(super) async fn delete_event(
        &self,
        target: &EventTarget,
        scope: Option<EditScope>,
    ) -> Result<Changed, CommandError> {
        let now = super::now();
        let (connection, _permit) = self.connect().await?;
        let before = held(&connection, target.occurrence.event).await?;
        let scope = scope_of(target, scope, before.recurs)?;
        let inverse = match postio_calendar::remove(&before.object, &scope).map_err(refused)? {
            Some(rest) => self.replace(&connection, &before, &rest, None, now).await?,
            None => {
                let change = record(
                    &connection,
                    Some(before.id),
                    before.calendar,
                    ChangeKind::Delete,
                    Some(&before),
                    now,
                )
                .await?;
                forget(&connection, before.id).await?;
                vec![revert(change)]
            }
        };
        tracing::info!(event = before.id.get(), "deleted an event");
        Ok(Changed {
            kind: UndoKind::DeleteEvent,
            inverse,
            lasts: None,
        })
    }

    pub(super) async fn move_event(
        &self,
        target: &EventTarget,
        start: DateTime<Utc>,
        scope: Option<EditScope>,
    ) -> Result<Changed, CommandError> {
        let now = super::now();
        let (connection, _permit) = self.connect().await?;
        let before = held(&connection, target.occurrence.event).await?;
        let scope = scope_of(target, scope, before.recurs)?;
        let edits = Edits {
            shift: Some(start - target.occurrence.start),
            ..Edits::default()
        };
        let edited = postio_calendar::patch(&before.object, &edits, &scope, &chrono::Local)
            .map_err(refused)?;
        let inverse = self
            .replace(
                &connection,
                &before,
                &edited.object,
                edited.split.as_deref(),
                now,
            )
            .await?;
        Ok(Changed {
            kind: UndoKind::MoveEvent,
            inverse,
            lasts: None,
        })
    }

    pub(super) async fn duplicate_event(&self, event: EventId) -> Result<Changed, CommandError> {
        let now = super::now();
        let (connection, _permit) = self.connect().await?;
        let calendars = CalendarRepository::new(&connection);
        let original = calendars
            .held(event)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That event is no longer here"))?;
        let calendar = if original.writable {
            original.calendar
        } else {
            calendars.local().await.map_err(store_failure)?.id
        };
        let copy = postio_calendar::with_uid(
            &original.object,
            &postio_model::outgoing::unique_at("postio.invalid"),
        )
        .map_err(refused)?;
        let added = put(&connection, calendar, &copy, None, now).await?;
        let change = record(
            &connection,
            Some(added),
            calendar,
            ChangeKind::Create,
            None,
            now,
        )
        .await?;
        Ok(Changed {
            kind: UndoKind::DuplicateEvent,
            inverse: vec![revert(change)],
            lasts: None,
        })
    }

    pub(super) async fn revert_event_change(
        &self,
        change: EventChangeId,
    ) -> Result<Changed, CommandError> {
        let now = super::now();
        let (connection, _permit) = self.connect().await?;
        let calendars = CalendarRepository::new(&connection);
        let change = calendars
            .change(change)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("There is nothing left to undo there"))?;
        let current = match change.event {
            Some(event) => calendars.held(event).await.map_err(store_failure)?,
            None => None,
        };
        match (change.kind, current, change.base) {
            (ChangeKind::Create, Some(current), _) => {
                record(
                    &connection,
                    Some(current.id),
                    current.calendar,
                    ChangeKind::Delete,
                    Some(&current),
                    now,
                )
                .await?;
                forget(&connection, current.id).await?;
            }
            (ChangeKind::Update, Some(current), Some(base)) => {
                put(&connection, current.calendar, &base, Some(&current), now).await?;
                record(
                    &connection,
                    Some(current.id),
                    current.calendar,
                    ChangeKind::Update,
                    Some(&current),
                    now,
                )
                .await?;
            }
            (ChangeKind::Delete, None, Some(base)) => {
                let back = put(&connection, change.calendar, &base, None, now).await?;
                record(
                    &connection,
                    Some(back),
                    change.calendar,
                    ChangeKind::Create,
                    None,
                    now,
                )
                .await?;
            }
            _ => return Err(CommandError::rejected("That event has changed since")),
        }
        Ok(Changed {
            kind: UndoKind::SaveEvent,
            inverse: Vec::new(),
            lasts: None,
        })
    }
}

fn invitation_text(fields: &EventFields, from: &Identity) -> String {
    let who = if from.display_name.trim().is_empty() {
        from.address.address.clone()
    } else {
        from.display_name.clone()
    };
    let at = fields
        .start
        .with_timezone(&chrono::Local)
        .format("%a %-d %b %Y, %H:%M");
    let place = if fields.location.trim().is_empty() {
        String::new()
    } else {
        format!(", {}", fields.location)
    };
    format!("{who} invites you to {}: {at}{place}.", fields.title)
}
