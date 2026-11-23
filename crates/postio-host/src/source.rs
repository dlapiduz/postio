use postio_model::MessageId;
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::{BlobStore, Store};
use postio_ui::source::{BodyState, Provider, RawState, SourceInput, SourceView};

pub struct Opened {
    pub view: SourceView,
    pub fetch: bool,
}

pub async fn view(
    database: &Store,
    blobs: &BlobStore,
    message: MessageId,
    absent: RawState,
) -> Result<Opened, String> {
    let connection = database.read().await.map_err(|error| error.to_string())?;
    let repository = MessageRepository::new(&connection);
    let row = repository
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("That message is no longer here")?;
    let body = if row.sync.body_state.has_body() {
        BodyState::Here
    } else {
        BodyState::Partial
    };
    let headers = match body {
        BodyState::Here => repository
            .body(message)
            .await
            .map_err(|error| error.to_string())?
            .and_then(|stored| stored.headers),
        BodyState::Partial => None,
    };
    let signature = signature(&connection, &row).await?;
    let account = AccountRepository::new(&connection)
        .get(row.account_id)
        .await
        .map_err(|error| error.to_string())?;
    drop(connection);
    let raw = row
        .raw_blob_id
        .as_ref()
        .and_then(|blob| blobs.get(blob).ok());
    let preset = account.as_ref().and_then(|account| {
        postio_account::discovery::preset_for_imap_host(&account.incoming.host)
    });
    let authserv_ids: Vec<&str> = preset
        .map(|preset| preset.authserv_ids().iter().map(String::as_str).collect())
        .unwrap_or_default();
    let sent_by_you = account.as_ref().is_some_and(|account| {
        std::iter::once(&account.address)
            .chain(account.identities.iter().map(|identity| &identity.address))
            .any(|own| row.from.iter().any(|from| from.same_address(own)))
    });
    let fetch = raw.is_none() && absent == RawState::Fetching;
    let here = raw.is_some();
    let envelope = envelope(&row);
    let view = postio_ui::source::view(SourceInput {
        subject: row.subject.as_deref().unwrap_or("(no subject)"),
        from: &row.from,
        headers: headers.as_deref(),
        envelope: Some(&envelope),
        raw,
        raw_state: if here { RawState::Here } else { absent },
        body,
        provider: preset.map(|preset| Provider {
            name: preset.display_name(),
            authserv_ids: &authserv_ids,
        }),
        sent_by_you,
        account: account
            .as_ref()
            .map_or("your account", |account| account.display_name.as_str()),
        signature,
    });
    Ok(Opened { view, fetch })
}

fn envelope(row: &postio_model::Message) -> String {
    let list = |addresses: &[postio_model::EmailAddress]| {
        addresses
            .iter()
            .map(postio_model::EmailAddress::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut block = Vec::new();
    for (name, value) in [
        ("From", list(&row.from)),
        ("To", list(&row.to)),
        ("Cc", list(&row.cc)),
        ("Subject", row.subject.clone().unwrap_or_default()),
        (
            "Date",
            row.date.map(|date| date.to_rfc2822()).unwrap_or_default(),
        ),
        (
            "Message-ID",
            row.rfc_message_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        ),
    ] {
        if !value.is_empty() {
            block.push(format!("{name}: {value}"));
        }
    }
    block.join("\r\n")
}

async fn signature(
    connection: &postio_storage::Checkout,
    row: &postio_model::Message,
) -> Result<Option<postio_ui::source::SignatureInput>, String> {
    let keys = postio_storage::repository::KeyRepository::new(connection);
    let Some(verdict) = keys
        .verdict(row.id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let Some(from) = row.from.first() else {
        return Ok(None);
    };
    let held = keys
        .for_address(from)
        .await
        .map_err(|error| error.to_string())?;
    let signer = held
        .iter()
        .find(|key| Some(&key.fingerprint) == verdict.fingerprint.as_ref());
    let sources = match signer {
        Some(key) => keys
            .sources(&key.fingerprint)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|source| format!("{source:?}").to_ascii_lowercase())
            .collect(),
        None => Vec::new(),
    };
    let accepted = keys
        .accepted(from)
        .await
        .map_err(|error| error.to_string())?
        .filter(|(key, _)| Some(key) == verdict.fingerprint.as_ref())
        .map(|(_, at)| at);
    Ok(Some(postio_ui::source::SignatureInput {
        status: verdict.status,
        reason: verdict.reason,
        scope: verdict.scope,
        fingerprint: verdict.fingerprint,
        sources,
        first_seen: signer.map(|key| key.first_seen),
        accepted_at: accepted,
        address: from.address.clone(),
    }))
}
