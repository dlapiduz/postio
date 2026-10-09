//! A conversation search's passages, cut from the corpus (spec 010 T028).
//!
//! The executor knows a body matched from which index found it, and nothing
//! about which *part* of the body: the person's own words, or the history
//! they quoted. That is decided here, when the passage is cut, with the same
//! quote detector the reader folds by (`postio_body::quote::text_stretches`)
//! over the same text the index holds (`indexable_text`) -- D6. So these run
//! over real mail from the `.eml` corpus, where the quoting is what clients
//! actually send, rather than over strings written to suit the rule.

use postio_model::{AccountScope, BodyState, MailboxId, MessageId};
use postio_search::results::{ConversationOrder, ConversationResults, Match, Source};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{Checkout, test_support};

/// One corpus message, stored with its body and indexed as sync would.
async fn store_fixture(
    connection: &Checkout,
    account: postio_model::AccountId,
    inbox: MailboxId,
    fixture: &str,
) -> MessageId {
    let parsed = postio_model::mime::parse(postio_model::test_corpus::load(fixture).bytes());
    let body = parsed.body.clone();
    let mut message = parsed.into_message(account, inbox, chrono::Utc::now());
    message.sync.body_state = BodyState::Full;
    let messages = MessageRepository::new(connection);
    messages.create(&mut message).await.expect("create");
    messages
        .set_body(
            message.id,
            &StoredBody {
                text: body.text.clone(),
                html: body.html.clone(),
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("store the body");
    let text = postio_index::index::indexable_text(&body);
    postio_index::index::index_body(connection, message.id.get(), text.as_deref())
        .await
        .expect("index the body");
    message.id
}

/// A message whose body is indexed but not on this machine: a real state,
/// in which the search finds it and nothing can be cut.
async fn store_bodiless(
    connection: &Checkout,
    account: postio_model::AccountId,
    inbox: MailboxId,
) -> MessageId {
    let mut message = postio_model::Message::new(account, inbox, chrono::Utc::now());
    message.subject = Some("Harbor survey".to_owned());
    message.from = vec![postio_model::EmailAddress::new(
        Some("Lena Park"),
        "lena@example.com",
    )];
    message.sync.body_state = BodyState::Full;
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    postio_index::index::index_body(
        connection,
        message.id.get(),
        Some("The soundings came back deeper than the chart says."),
    )
    .await
    .expect("index the body");
    message.id
}

struct World {
    _database: test_support::TempStore,
    connection: Checkout,
    account: postio_model::AccountId,
    inbox: MailboxId,
    reply: MessageId,
    flowed: MessageId,
    bodiless: MessageId,
}

async fn world() -> World {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let reply = store_fixture(&connection, account.id, inbox, "top-posted-reply-signature").await;
    let flowed = store_fixture(&connection, account.id, inbox, "plain-text-flowed-reply").await;
    let bodiless = store_bodiless(&connection, account.id, inbox).await;
    World {
        _database: database,
        connection,
        account: account.id,
        inbox,
        reply,
        flowed,
        bodiless,
    }
}

fn parse(text: &str) -> postio_search::ParsedQuery {
    postio_search::parse(text, chrono::Utc::now().date_naive())
}

async fn search(world: &World, text: &str) -> ConversationResults {
    postio_session::search::conversations(
        &world.connection,
        AccountScope::Account(world.account),
        &parse(text),
        ConversationOrder::BestMatch,
        0,
        20,
    )
    .await
    .expect("the search runs")
}

/// The one hit `text` finds, and where the executor says it matched.
async fn only_hit(world: &World, text: &str) -> (MessageId, Vec<Source>) {
    let results = search(world, text).await;
    assert_eq!(results.hits.len(), 1, "`{text}` finds one conversation");
    let hit = &results.hits[0];
    (
        hit.best,
        hit.matches
            .iter()
            .map(|found| found.source.clone())
            .collect(),
    )
}

async fn passages(world: &World, text: &str, hit: (MessageId, Vec<Source>)) -> Vec<Match> {
    let id = hit.0;
    let mut answered = postio_session::search::passages(
        &world.connection,
        &parse(text),
        &[hit],
        postio_search::passage::FirstLine::Shown,
    )
    .await;
    assert_eq!(answered.len(), 1, "one answer per hit asked about");
    let (answered_id, matches) = answered.remove(0);
    assert_eq!(answered_id, id, "answered for the hit asked about");
    matches
}

fn highlighted(found: &Match) -> Vec<String> {
    let passage = found.passage.as_ref().expect("a passage");
    passage
        .ranges
        .iter()
        .map(|range| passage.text[range.clone()].to_lowercase())
        .collect()
}

#[tokio::test]
async fn a_word_only_in_quoted_history_is_tagged_quoted_with_the_quote_as_its_passage() {
    let world = world().await;
    let hit = only_hit(&world, "copies").await;
    assert_eq!(hit.0, world.reply);
    assert_eq!(
        hit.1,
        vec![Source::Body],
        "the index knows only that the body matched"
    );

    let matches = passages(&world, "copies", hit).await;
    assert_eq!(
        matches
            .iter()
            .map(|found| &found.source)
            .collect::<Vec<_>>(),
        vec![&Source::Quoted],
        "the quote is where it matched, and nothing is said about the body"
    );
    assert_eq!(highlighted(&matches[0]), vec!["copies"]);
    let text = &matches[0].passage.as_ref().expect("a passage").text;
    assert!(
        text.contains("sign both copies"),
        "cut from the quoted words: {text:?}"
    );
    assert!(
        !text.contains('>'),
        "the quote's markers are not part of what was said: {text:?}"
    );
}

#[tokio::test]
async fn a_word_only_in_the_persons_own_words_is_tagged_body() {
    let world = world().await;
    let hit = only_hit(&world, "initial").await;
    assert_eq!(hit.0, world.reply);

    let matches = passages(&world, "initial", hit).await;
    assert_eq!(
        matches
            .iter()
            .map(|found| &found.source)
            .collect::<Vec<_>>(),
        vec![&Source::Body]
    );
    assert_eq!(highlighted(&matches[0]), vec!["initial"]);
    let text = &matches[0].passage.as_ref().expect("a passage").text;
    assert!(text.contains("initial page four"), "{text:?}");
}

#[tokio::test]
async fn a_word_in_both_is_tagged_once_for_each() {
    // "lease" is in the subject, in Quinn's own reply and in Ada's quoted
    // message: three places, each named once.
    let world = world().await;
    let hit = only_hit(&world, "lease").await;
    assert_eq!(hit.0, world.reply);
    assert_eq!(hit.1, vec![Source::Subject, Source::Body]);

    let matches = passages(&world, "lease", hit).await;
    assert_eq!(
        matches
            .iter()
            .map(|found| &found.source)
            .collect::<Vec<_>>(),
        vec![&Source::Subject, &Source::Body, &Source::Quoted]
    );
    assert!(highlighted(&matches[1]).contains(&"lease".to_owned()));
    assert!(highlighted(&matches[2]).contains(&"lease".to_owned()));
}

#[tokio::test]
async fn a_word_in_the_subject_is_tagged_subject() {
    let world = world().await;
    let hit = only_hit(&world, "tuesday").await;
    assert_eq!(hit.0, world.flowed);
    assert_eq!(hit.1, vec![Source::Subject]);

    let matches = passages(&world, "tuesday", hit).await;
    assert_eq!(
        matches
            .iter()
            .map(|found| &found.source)
            .collect::<Vec<_>>(),
        vec![&Source::Subject],
        "the row draws the subject itself; the body was not read into a source"
    );
}

#[tokio::test]
async fn a_message_with_no_local_body_keeps_its_sources_without_passages() {
    let world = world().await;
    let hit = only_hit(&world, "soundings").await;
    assert_eq!(hit.0, world.bodiless);
    assert_eq!(hit.1, vec![Source::Body]);

    let matches = passages(&world, "soundings", hit).await;
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].source,
        Source::Body,
        "still the body that matched"
    );
    assert_eq!(matches[0].passage, None, "nothing here to cut it from");
}

#[tokio::test]
async fn the_facets_ids_come_back_with_their_names() {
    let world = world().await;
    // Both of Quinn's replies say "Ada"; the bodiless message does not.
    let results = search(&world, "ada").await;
    assert_eq!(results.hits.len(), 2);
    assert!(!results.facets.senders.is_empty());

    for count in results
        .facets
        .senders
        .iter()
        .chain(&results.facets.recipients)
    {
        assert!(
            results.names.people.iter().any(|(id, _)| *id == count.id),
            "every person counted is named: {:?}",
            count.id
        );
    }
    let quinn = results
        .names
        .people
        .iter()
        .find(|(_, address)| address.address == "quinn.abara@example.net")
        .map(|(_, address)| address.clone())
        .expect("Quinn sent two of these");
    assert_eq!(quinn.name.as_deref(), Some("Quinn Abara"));

    assert_eq!(
        results.names.folders,
        vec![(world.inbox, "INBOX".to_owned())],
        "the one folder, by the name the store gave it"
    );
}

/// The line that introduces a quote ("On 21/09/2026 16:02, Ada Norwood
/// wrote:") is neither the person's words nor the history: a passage is
/// never cut from it, even when it holds the word searched for. "Ada" is
/// in Quinn's greeting, in that line and in the quote's signature.
#[tokio::test]
async fn a_passage_is_never_the_line_that_introduces_a_quote() {
    let world = world().await;
    let results = search(&world, "ada").await;
    let hit = results
        .hits
        .iter()
        .find(|hit| hit.best == world.reply)
        .expect("the top-posted reply says Ada");
    let asked = (
        hit.best,
        hit.matches
            .iter()
            .map(|found| found.source.clone())
            .collect(),
    );

    let matches = passages(&world, "ada", asked).await;
    let cut: Vec<&str> = matches
        .iter()
        .filter_map(|found| found.passage.as_ref())
        .map(|passage| passage.text.as_str())
        .collect();
    assert!(!cut.is_empty(), "{matches:?}");
    for text in cut {
        assert!(
            !text.contains("wrote:"),
            "cut from the quote's attribution: {text:?}"
        );
    }
}
