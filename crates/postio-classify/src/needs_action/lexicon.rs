//! The detector's words, compiled in: the small table of rules FR-165 allows
//! the built-in detector, which needs no inference engine. English first
//! (spec, Assumptions).
//!
//! None of it names a sender. Who sent a message is the automated-senders
//! table's to say, as data (FR-114); these are words people write.

/// Words that put a sentence to the reader: R10's second person.
pub(super) const SECOND_PERSON: &[&str] = &[
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    "you're",
    "you've",
    "you'll",
    "you'd",
];

/// Words that open a clause without naming anybody: greetings, and the
/// words that come before a comma ("Anyway, please send..."). A capitalised
/// one is not a name.
pub(super) const OPENERS: &[&str] = &[
    "hi",
    "hey",
    "hello",
    "dear",
    "morning",
    "hiya",
    "evening",
    "afternoon",
    "good",
    "thanks",
    "thank",
    "also",
    "so",
    "anyway",
    "ok",
    "okay",
    "well",
    "sorry",
    "yes",
    "no",
    "sure",
    "great",
    "first",
    "second",
    "third",
    "quick",
    "one",
    "oh",
    "btw",
    "ps",
    "fyi",
    "finally",
    "lastly",
    "otherwise",
    "meanwhile",
    "separately",
    "however",
    "and",
    "but",
    "then",
    "now",
    "done",
    "noted",
    "agreed",
    "perfect",
    "cool",
    "right",
    "yep",
    "yeah",
    "update",
    "reminder",
    "question",
    "note",
];

/// Words a salutation uses for everyone it is sent to, the reader among
/// them: "Hi all", "Hi both".
pub(super) const GROUPS: &[&str] = &[
    "all",
    "everyone",
    "everybody",
    "team",
    "both",
    "folks",
    "guys",
    "friends",
    "colleagues",
    "there",
];

/// Capitalised words that are not somebody's name.
pub(super) const NOT_NAMES: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "please",
];

/// Openings that ask the reader to act (R10).
pub(super) const REQUEST_OPENINGS: &[&str] = &["can you", "could you", "would you"];

/// Asks wherever they stand in the sentence (R10): "let me know", "I need
/// you to".
pub(super) const ASKS: &[&str] = &["let me know", "need you to"];

/// The words before "please" that keep it a request: it opens the clause,
/// or follows one of these ("could you please", "and please").
pub(super) const BEFORE_PLEASE: &[&str] = &[
    "and", "also", "so", "but", "now", "then", "just", "kindly", "you",
];

/// A stated need, which is an ask when it names a deadline: "I need the
/// signed form by Wednesday".
pub(super) const NEEDS: &[&str] = &[
    "i need",
    "we need",
    "i'll need",
    "we'll need",
    "i will need",
    "we will need",
];

/// Verbs that open an imperative (R10).
pub(super) const IMPERATIVES: &[&str] = &[
    "send", "review", "sign", "approve", "confirm", "fill", "complete", "submit", "update", "book",
    "bring", "check", "read", "add", "call", "reply", "forward", "upload", "share", "schedule",
    "return", "register", "rsvp", "pay", "renew", "finish", "prepare", "draft", "email", "ping",
    "pick", "leave", "move", "fix", "test", "look", "verify",
];

/// The word after an imperative verb that shows it is one: an object or a
/// particle. "Send me", "Book a room"; not "Book club is on Thursday".
pub(super) const OBJECTS: &[&str] = &[
    "the",
    "a",
    "an",
    "this",
    "that",
    "these",
    "those",
    "me",
    "us",
    "it",
    "them",
    "him",
    "her",
    "my",
    "your",
    "our",
    "their",
    "his",
    "its",
    "some",
    "any",
    "all",
    "both",
    "each",
    "every",
    "everything",
    "anything",
    "something",
    "in",
    "out",
    "up",
    "back",
    "over",
    "through",
    "on",
    "off",
    "down",
    "by",
    "to",
    "for",
    "with",
    "at",
    "from",
    "into",
    "around",
    "about",
    "what",
    "which",
    "whether",
];

/// Imperatives that are phrases rather than one verb (R10).
pub(super) const IMPERATIVE_PHRASES: &[&str] = &[
    "don't forget",
    "do not forget",
    "make sure",
    "remember to",
    "be sure to",
];

/// An imperative that recommends rather than asks: "Check out this
/// article" (research R10, "What still goes wrong").
pub(super) const RECOMMENDATIONS: &[&str] = &["check out", "check it out", "check this out"];

/// Small talk, which R10 excludes by name.
pub(super) const PLEASANTRIES: &[&str] = &[
    "how are you",
    "how are things",
    "how's it going",
    "how is it going",
    "how have you been",
    "how's life",
    "hope you're well",
    "hope you are well",
    "hope all is well",
    "hope all's well",
    "hope everything is",
    "did you enjoy",
    "did you have fun",
];

/// Asking after how things are or went is small talk, not a question to
/// act on: "How's the new job treating you?", "How was the trip?".
pub(super) const SMALL_TALK_OPENINGS: &[&str] = &[
    "how's", "how is", "how are", "how was", "how were", "how have", "how has",
];

/// Asking whether the reader had a good time is small talk too: these,
/// followed by one of [`PLEASANT`].
pub(super) const HAD_A: &[&str] = &["did you have a", "hope you had a", "hope you've had a"];

/// What a good time is called.
pub(super) const PLEASANT: &[&str] = &[
    "good",
    "nice",
    "great",
    "lovely",
    "fun",
    "wonderful",
    "relaxing",
    "restful",
    "fantastic",
    "brilliant",
    "safe",
    "pleasant",
];

/// Questions that expect no answer, which R10 excludes by name.
pub(super) const RHETORICAL: &[&str] = &[
    "who knew",
    "can you believe",
    "can you imagine",
    "what could go wrong",
    "what could possibly go wrong",
    "right?",
    "isn't it",
    "don't you think",
    "guess what",
    "why not",
    "who would have thought",
    "you know?",
];

/// Boilerplate that reads as a request and is not one (R10's first fix).
pub(super) const BOILERPLATE: &[&str] = &[
    "let me know if you have any",
    "let me know if you have questions",
    "let me know if you need anything",
    "let me know if there's anything",
    "let me know if there is anything",
    "if you have any questions",
    "please find attached",
    "please find enclosed",
    "please see attached",
    "please see below",
    "see below",
    "see attached",
    "don't hesitate",
    "do not hesitate",
    "feel free",
    "please note",
    "please ignore",
    "please disregard",
    "please consider the environment",
    "received it in error",
    "received this in error",
    "received this email in error",
    "received this message in error",
    "don't reply",
    "do not reply",
];

/// Who text is addressed to when it is not the person reading (R10's third
/// fix): an ask put to an assistant is never the reader's.
pub(super) const ASSISTANT: &[&str] = &[
    "assistant",
    "ai",
    "language model",
    "language models",
    "llm",
    "chatbot",
    "automated agent",
    "ai agent",
    "previous instructions",
    "prior instructions",
    "system prompt",
];

/// What only text aimed at a model says (ADR 0009 Q4): an instruction about
/// its instructions, a name for the machine, or a tool call. A message
/// holding any of it is asked nothing at all. Unlike [`ASSISTANT`], nothing
/// here is a word ordinary mail uses: a person's assistant is not in it, and
/// nor is "AI" alone.
pub(super) const TO_A_MACHINE: &[&str] = &[
    "previous instructions",
    "prior instructions",
    "system prompt",
    "ai assistant",
    "email assistant",
    "dear ai",
    "language model",
    "automated agent",
    "ai agent",
    "tool_call",
    "function_call",
];

/// What makes a phrase a deadline rather than a date (R10): "by Friday",
/// "before 15 October", "no later than Thursday". "On Monday" says when, not
/// by when.
pub(super) const DEADLINE_KEYWORDS: &[&str] = &["by", "before", "no later than"];

/// Words that open the name of an event a deadline is set by: "before the
/// release on Friday", "before our call on Tuesday".
pub(super) const EVENT_OPENERS: &[&str] = &["the", "our", "your", "my", "this", "their", "next"];

/// Deadlines `parse_when` has no word for, as the words it has: each maps a
/// phrase to the day it ends on.
pub(super) enum Idiom {
    /// Today, by its evening.
    Tonight,
    /// This week's Friday, by its evening.
    Friday,
    /// This month's last day, by its evening.
    MonthEnd,
}

/// The phrases, as their words.
pub(super) const IDIOMS: &[(&[&str], Idiom)] = &[
    (&["end", "of", "day"], Idiom::Tonight),
    (&["end", "of", "the", "day"], Idiom::Tonight),
    (&["eod"], Idiom::Tonight),
    (&["cob"], Idiom::Tonight),
    (&["close", "of", "business"], Idiom::Tonight),
    (&["end", "of", "business"], Idiom::Tonight),
    (&["close", "of", "play"], Idiom::Tonight),
    (&["today"], Idiom::Tonight),
    (&["tonight"], Idiom::Tonight),
    (&["end", "of", "week"], Idiom::Friday),
    (&["end", "of", "the", "week"], Idiom::Friday),
    (&["end", "of", "this", "week"], Idiom::Friday),
    (&["eow"], Idiom::Friday),
    (&["end", "of", "month"], Idiom::MonthEnd),
    (&["end", "of", "the", "month"], Idiom::MonthEnd),
    (&["end", "of", "this", "month"], Idiom::MonthEnd),
    (&["eom"], Idiom::MonthEnd),
];

/// Abbreviations whose full stop does not end a sentence.
pub(super) const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "approx", "cf", "mr", "mrs", "ms", "dr",
];
