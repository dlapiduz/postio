//! The threads seed (spec 013 T001): the conversations the threads design
//! draws, filed through the real threading path.
//!
//! Nothing here names a thread: each message carries its `Message-ID`,
//! `In-Reply-To` and `References` and goes through
//! `ThreadingRepository::thread`, so the grouping the screens show is the
//! engine's own. Every name is invented and every address is on `example.com`
//! or `example.net`.

use chrono::{DateTime, Duration, Utc};
use postio_model::{
    AccountId, Attachment, Draft, DraftKind, EmailAddress, Flag, FlagSet, MailboxId, Message,
    MessageId, RfcMessageId,
};
use postio_storage::repository::{
    DraftRepository, MessageRepository, StoredBody, ThreadingRepository,
};
use postio_storage::{Connection, Store};

use crate::{record_correspondents, synced, today};

/// The people of the seed, by index. Index 0 is the person at the keyboard.
const PEOPLE: [(&str, &str); 9] = [
    ("You", "you@example.com"),
    ("Ana Ruiz", "ana@example.com"),
    ("Lena Park", "lena@example.com"),
    ("Ben Adeyemi", "ben@example.com"),
    ("Tomás Reyes", "tomas@example.com"),
    ("Grace Oyelaran", "grace@example.com"),
    ("Priya Nair", "priya@example.com"),
    ("Marcus Webb", "marcus@example.com"),
    ("Sofia Lindqvist", "sofia@example.com"),
];

const YOU: usize = 0;
const ANA: usize = 1;
const LENA: usize = 2;
const BEN: usize = 3;
const TOMAS: usize = 4;
const GRACE: usize = 5;
const PRIYA: usize = 6;
const MARCUS: usize = 7;

/// Neighbours on the mailing list: names and addresses on `example.net`.
const NEIGHBOURS: [(&str, &str); 8] = [
    ("Mira Tan", "mira@example.net"),
    ("Odile Brandt", "odile@example.net"),
    ("Kofi Mensah", "kofi@example.net"),
    ("Ingrid Salo", "ingrid@example.net"),
    ("Joaquín Vega", "joaquin@example.net"),
    ("Nell Fairbairn", "nell@example.net"),
    ("Dara Osei", "dara@example.net"),
    ("Pavel Novak", "pavel@example.net"),
];

/// Who a message is from.
#[derive(Clone, Copy)]
enum Who {
    Person(usize),
    Neighbour(usize),
}

impl Who {
    fn address(self) -> EmailAddress {
        let (name, address) = match self {
            Who::Person(index) => PEOPLE[index],
            Who::Neighbour(index) => NEIGHBOURS[index],
        };
        EmailAddress::new(Some(name), address)
    }

    fn is_you(self) -> bool {
        matches!(self, Who::Person(YOU))
    }
}

/// One message to file.
struct Turn {
    from: Who,
    at: DateTime<Utc>,
    text: String,
    unread: bool,
    /// The turn this one answers; the previous one when `None`.
    answers: Option<usize>,
    /// A file named on it: the part is declared, its bytes are not here.
    file: Option<(&'static str, u64)>,
}

fn turn(from: Who, at: DateTime<Utc>, text: &str) -> Turn {
    Turn {
        from,
        at,
        text: text.to_owned(),
        unread: false,
        answers: None,
        file: None,
    }
}

struct Thread {
    /// Its key in the `Message-ID`s.
    key: &'static str,
    subject: &'static str,
    /// `Re:` on every message, as a list's replies are.
    always_re: bool,
    list_id: Option<&'static str>,
    turns: Vec<Turn>,
}

/// `count` turns going back from `last`, `gap` apart, from `speakers` in
/// rotation, saying the sentences of `lines` in rotation. The newest
/// `unread` are unread.
fn run(
    count: usize,
    last: DateTime<Utc>,
    gap: Duration,
    speakers: &[Who],
    lines: &[&str],
    unread: usize,
) -> Vec<Turn> {
    (0..count)
        .map(|index| {
            let mut made = turn(
                speakers[index % speakers.len()],
                last - gap * (count - 1 - index) as i32,
                &format!("{} ({})", lines[index % lines.len()], index + 1),
            );
            made.unread = index + unread >= count;
            made
        })
        .collect()
}

fn conversations(today: DateTime<Utc>) -> Vec<Thread> {
    let p = Who::Person;
    let day = Duration::days;
    let hour = Duration::hours;
    let mut harbor = vec![
        turn(
            p(ANA),
            today - day(5),
            "Here is the first cut of the Harbor API draft. Auth, pagination and errors are \
             in; webhooks are still open.",
        ),
        turn(
            p(LENA),
            today - day(4),
            "Read it twice. The pagination section is clear. Can we say what happens when a \
             cursor expires?",
        ),
        turn(
            p(YOU),
            today - day(4) + hour(3),
            "Cursors expire after fifteen minutes. We should also say whether we send the \
             rate limit headers on every response or only on success.",
        ),
        turn(
            p(BEN),
            today - day(2),
            "Ana, on the webhooks: I would sign every delivery and leave retries to the \
             receiver for v3.",
        ),
        turn(
            p(LENA),
            today - Duration::minutes(47),
            "Draft v3 attached. I added the retry table and moved the rate limit headers to \
             their own section.",
        ),
        turn(
            p(BEN),
            today - Duration::minutes(7),
            "Looks good. One question on the rate-limit headers: do we send them on 4xx too?",
        ),
    ];
    harbor[3].answers = Some(0);
    harbor[4].file = Some(("Harbor-API-v3.pdf", 214_000));
    harbor[4].unread = true;
    harbor[5].unread = true;

    let offsite = run(
        23,
        today - hour(2),
        Duration::minutes(97),
        &[p(TOMAS), p(ANA), p(BEN), p(YOU), p(PRIYA), p(MARCUS)],
        &[
            "Can we settle the dates for the offsite this week?",
            "Two nights is enough; three is a long way to carry a team.",
            "I priced the coastal option: it fits the budget with a day to spare.",
            "Who is driving and who is flying?",
            "Dietary notes are in the shared sheet, please add yours.",
            "The venue wants a deposit by Friday.",
        ],
        3,
    );

    let cat = run(
        41,
        today - hour(1),
        Duration::minutes(53),
        &(0..8).map(Who::Neighbour).collect::<Vec<_>>(),
        &[
            "Has anyone seen a grey tabby near the allotments? Answers to Biscuit.",
            "Saw one by the bakery bins at six this morning, no collar.",
            "Biscuit has a notched left ear; that may help.",
            "Checked the sheds on Mill Lane, nothing yet.",
            "Posted flyers at the post office and the bus shelter.",
            "She is microchipped, so the vets will know if she comes in.",
        ],
        0,
    );

    let slos = {
        let mut turns = run(
            8,
            today - day(3),
            hour(5),
            &[p(MARCUS), p(PRIYA), p(YOU), p(BEN)],
            &[
                "Latency targets for the gateway are in the doc, comments welcome.",
                "Is 99.9 per cent monthly the right availability target?",
                "The error budget policy needs an owner.",
                "Burn alerts should page only at the two-hour window.",
            ],
            0,
        );
        turns[4].text =
            "We should publish the rate limit headers in the SLO dashboard too.".to_owned();
        turns
    };

    vec![
        Thread {
            key: "harbor",
            subject: "Harbor API draft v3",
            always_re: false,
            list_id: None,
            turns: harbor,
        },
        Thread {
            key: "offsite",
            subject: "Offsite planning",
            always_re: false,
            list_id: None,
            turns: offsite,
        },
        Thread {
            key: "cat",
            subject: "Re: lost cat",
            always_re: true,
            list_id: Some("lost-cat.lists.example.net"),
            turns: cat,
        },
        Thread {
            key: "slos",
            subject: "Re: Platform SLOs",
            always_re: true,
            list_id: None,
            turns: slos,
        },
        Thread {
            key: "gateway",
            subject: "Gateway defaults",
            always_re: false,
            list_id: None,
            turns: vec![turn(
                p(BEN),
                today - day(6),
                "Proposed gateway defaults: 100 requests a minute, burst of 20, and the rate \
                 limit headers on every response.",
            )],
        },
        Thread {
            key: "design-review",
            subject: "Invitation: Harbor design review",
            always_re: false,
            list_id: None,
            turns: vec![{
                let mut invite = turn(
                    p(GRACE),
                    today - hour(4),
                    "Grace Oyelaran invited you to the Harbor design review, Tuesday at 10:00.",
                );
                invite.unread = true;
                invite
            }],
        },
        Thread {
            key: "staffing",
            subject: "Atlas staffing plan for Q4",
            always_re: false,
            list_id: None,
            turns: vec![{
                let mut plan = turn(
                    p(TOMAS),
                    today - hour(9),
                    "Staffing plan for Q4 attached in the body below: two engineers join in \
                     October, one rotates off in December.",
                );
                plan.unread = true;
                plan
            }],
        },
        Thread {
            key: "budget",
            subject: "Re: Atlas Q3 budget, final numbers",
            always_re: true,
            list_id: None,
            turns: run(
                4,
                today - day(2) - hour(3),
                hour(6),
                &[p(TOMAS), p(YOU)],
                &[
                    "Final numbers for the Atlas Q3 budget are in.",
                    "Thanks. The contractor line looks high; is that a typo?",
                ],
                0,
            ),
        },
        Thread {
            key: "sow",
            subject: "Re: Harbor SOW",
            always_re: true,
            list_id: None,
            turns: run(
                3,
                today - day(1) - hour(2),
                hour(8),
                &[p(GRACE), p(YOU)],
                &[
                    "The SOW needs a signature by month end.",
                    "Signed and sent back.",
                ],
                0,
            ),
        },
        Thread {
            key: "dinner",
            subject: "Re: dinner Saturday?",
            always_re: true,
            list_id: None,
            turns: run(
                2,
                today - day(1),
                hour(2),
                &[p(PRIYA), p(YOU)],
                &[
                    "Dinner on Saturday? Seven at the usual place.",
                    "Yes, see you there.",
                ],
                0,
            ),
        },
    ]
}

fn rfc_id(key: &str, turn: usize) -> RfcMessageId {
    RfcMessageId::new(format!("<threads.{key}.{turn}@example.test>"))
}

/// The threads seed's store and its account.
pub async fn threads_demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
    postio_storage::repository::IdentityRepository::new(&connection)
        .create(&mut postio_model::Identity::new(
            account.id,
            EmailAddress::new(Some("You"), PEOPLE[YOU].1),
        ))
        .await
        .expect("your identity");
    let today = today();
    synced(&connection, inbox, today).await;
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search index");

    let mut bodies: Vec<(MessageId, String)> = Vec::new();
    let mut harbor_reply = None;
    for thread in conversations(today) {
        let last = file_thread(&connection, account.id, inbox, &thread, &mut bodies).await;
        if thread.key == "harbor" {
            harbor_reply = Some(last);
        }
    }

    // Harbor's unsent reply to Ben's question.
    let (thread_id, answering) = harbor_reply.expect("harbor was filed");
    let mut draft = Draft::new(account.id);
    draft.kind = DraftKind::Reply;
    draft.in_reply_to = Some(answering);
    draft.thread_id = Some(thread_id);
    draft.subject = "Re: Harbor API draft v3".to_owned();
    draft.to = vec![Who::Person(BEN).address()];
    draft.body.text =
        Some("Thanks both. I'll review tonight and come back on the 4xx question.".to_owned());
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("the draft saves");

    // Indexed as the body indexer would have by now, in one transaction.
    postio_storage::sql::batch(&connection, "BEGIN")
        .await
        .expect("begin the bodies");
    for (message, body) in &bodies {
        postio_index::index::index_body(&connection, message.get(), Some(body))
            .await
            .expect("a body indexed");
    }
    postio_storage::sql::batch(&connection, "COMMIT")
        .await
        .expect("commit the bodies");
    drop(connection);
    (database, account.id)
}

/// Files every turn of `thread`; the thread's id and its newest message.
async fn file_thread(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    thread: &Thread,
    bodies: &mut Vec<(MessageId, String)>,
) -> (postio_model::ThreadId, MessageId) {
    let mut filed: Vec<MessageId> = Vec::new();
    let mut thread_id = None;
    for (index, turn) in thread.turns.iter().enumerate() {
        let parent = turn.answers.or(index.checked_sub(1));
        let mut message = Message::new(account, mailbox, turn.at);
        message.date = Some(turn.at);
        message.from = vec![turn.from.address()];
        message.to = vec![if turn.from.is_you() {
            parent
                .map_or(Who::Person(ANA), |parent| thread.turns[parent].from)
                .address()
        } else {
            Who::Person(YOU).address()
        }];
        message.subject = Some(
            if index == 0 || thread.always_re || thread.subject.starts_with("Re: ") {
                thread.subject.to_owned()
            } else {
                format!("Re: {}", thread.subject)
            },
        );
        message.list_id = thread.list_id.map(str::to_owned);
        message.preview = Some(turn.text.chars().take(110).collect());
        let mut flags = FlagSet::new();
        if !turn.unread {
            flags.insert(Flag::Seen);
        }
        message.flags = flags;
        if let Some((name, size)) = turn.file {
            let mut file = Attachment::new(MessageId::UNASSIGNED, "application/pdf", size);
            file.filename = Some(name.to_owned());
            file.part_id = Some("2".to_owned());
            message.attachments.push(file);
        }
        message.rfc_message_id = Some(rfc_id(thread.key, index));
        if let Some(parent) = parent {
            // The chain back to the first message, nearest last.
            let mut chain = vec![parent];
            let mut at = parent;
            while let Some(up) = thread.turns[at].answers.or(at.checked_sub(1)) {
                chain.push(up);
                at = up;
            }
            chain.reverse();
            message.in_reply_to = Some(rfc_id(thread.key, parent));
            message.references = chain.into_iter().map(|n| rfc_id(thread.key, n)).collect();
        }
        MessageRepository::new(connection)
            .create(&mut message)
            .await
            .expect("a message");
        record_correspondents(connection, &message)
            .await
            .expect("its correspondents");
        let threaded = ThreadingRepository::new(connection, account)
            .thread(&message)
            .await
            .expect("threaded");
        thread_id = Some(threaded.thread_id);
        let body = match parent {
            Some(parent) => {
                let quote: String = thread.turns[parent]
                    .text
                    .lines()
                    .map(|line| format!("> {line}\n"))
                    .collect();
                format!(
                    "{}\n\nOn {}, {} wrote:\n{quote}",
                    turn.text,
                    thread.turns[parent].at.format("%a %-d %b %Y at %H:%M"),
                    name_of(thread.turns[parent].from),
                )
            }
            None => format!("{}\n", turn.text),
        };
        bodies.push((message.id, body.clone()));
        MessageRepository::new(connection)
            .set_body(
                message.id,
                &StoredBody {
                    text: Some(body),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
        filed.push(message.id);
    }
    (
        thread_id.expect("a thread has a message"),
        *filed.last().expect("a thread has a message"),
    )
}

#[allow(non_snake_case)]
fn name_of(who: Who) -> &'static str {
    match who {
        Who::Person(index) => PEOPLE[index].0,
        Who::Neighbour(index) => NEIGHBOURS[index].0,
    }
}

/// What a reply's arrival leaves for the caller to announce, as a sync pass
/// would: `NewMail` for the message and `ThreadChanged` for its thread.
#[derive(Debug, Clone, Copy)]
pub struct Arrival {
    /// The mailbox it landed in.
    pub mailbox: MailboxId,
    /// The thread it joined.
    pub thread: postio_model::ThreadId,
    /// The message.
    pub message: MessageId,
}

/// Ana's reply in the Harbor thread (spec 013 US4, screen 08): written,
/// threaded through the engine's own path (`In-Reply-To` and `References`
/// from Ben's last message), with its body and the inbox counted. Delivered
/// again, it is another reply, one minute on.
pub async fn deliver_harbor_reply(store: &Store, account: AccountId) -> Result<Arrival, String> {
    let text = |error: postio_storage::Error| error.to_string();
    let connection = store.connect().await.map_err(|error| error.to_string())?;
    let messages = MessageRepository::new(&connection);
    let inbox = postio_storage::repository::MailboxRepository::new(&connection)
        .by_role(account, postio_model::MailboxRole::Inbox)
        .await
        .map_err(text)?
        .ok_or("the account has no inbox")?
        .id;
    // The newest member answers the reply: the chain of rfc ids is what
    // the threading pass links by.
    let mut chain: Vec<RfcMessageId> = (0..6).map(|turn| rfc_id("harbor", turn)).collect();
    let mut sent = 0;
    while !messages
        .ids_by_rfc_message_id(
            account,
            &RfcMessageId::new(format!("<threads.harbor.reply{sent}@example.test>")),
        )
        .await
        .map_err(text)?
        .is_empty()
    {
        chain.push(RfcMessageId::new(format!(
            "<threads.harbor.reply{sent}@example.test>"
        )));
        sent += 1;
    }
    let at = today() + Duration::minutes(sent as i64);
    let mut message = Message::new(account, inbox, at);
    message.date = Some(at);
    message.from = vec![Who::Person(ANA).address()];
    message.to = vec![Who::Person(YOU).address()];
    message.subject = Some("Re: Harbor API draft v3".to_owned());
    let line = "+1 to Ben on exports. Lena, can you fold that in?";
    message.preview = Some(line.to_owned());
    message.rfc_message_id = Some(RfcMessageId::new(format!(
        "<threads.harbor.reply{sent}@example.test>"
    )));
    message.in_reply_to = chain.last().cloned();
    message.references = chain;
    messages.create(&mut message).await.map_err(text)?;
    record_correspondents(&connection, &message)
        .await
        .map_err(text)?;
    let threaded = ThreadingRepository::new(&connection, account)
        .thread(&message)
        .await
        .map_err(text)?;
    messages
        .set_body(
            message.id,
            &StoredBody {
                text: Some(format!("{line}\n")),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .map_err(text)?;
    postio_storage::repository::MailboxRepository::new(&connection)
        .recount(inbox)
        .await
        .map_err(text)?;
    Ok(Arrival {
        mailbox: inbox,
        thread: threaded.thread_id,
        message: message.id,
    })
}
