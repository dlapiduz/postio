//! The detector's words, compiled in: a small rule table that needs no inference engine. None names
//! a sender; that is the automated-senders table's, as data.

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

/// Words that open a clause without naming anybody. A capitalised one is not a name.
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

pub(super) const REQUEST_OPENINGS: &[&str] = &["can you", "could you", "would you"];

pub(super) const ASKS: &[&str] = &["let me know", "need you to"];

pub(super) const BEFORE_PLEASE: &[&str] = &[
    "and", "also", "so", "but", "now", "then", "just", "kindly", "you",
];

pub(super) const NEEDS: &[&str] = &[
    "i need",
    "we need",
    "i'll need",
    "we'll need",
    "i will need",
    "we will need",
];

pub(crate) const IMPERATIVES: &[&str] = &[
    "send", "review", "sign", "approve", "confirm", "fill", "complete", "submit", "update", "book",
    "bring", "check", "read", "add", "call", "reply", "forward", "upload", "share", "schedule",
    "return", "register", "rsvp", "pay", "renew", "finish", "prepare", "draft", "email", "ping",
    "pick", "leave", "move", "fix", "test", "look", "verify",
];

/// The word after an imperative verb that shows it is one: "Send me", not "Book club is on
/// Thursday".
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

pub(super) const IMPERATIVE_PHRASES: &[&str] = &[
    "don't forget",
    "do not forget",
    "make sure",
    "remember to",
    "be sure to",
];

pub(super) const RECOMMENDATIONS: &[&str] = &["check out", "check it out", "check this out"];

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

pub(super) const SMALL_TALK_OPENINGS: &[&str] = &[
    "how's", "how is", "how are", "how was", "how were", "how have", "how has",
];

pub(super) const HAD_A: &[&str] = &["did you have a", "hope you had a", "hope you've had a"];

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

/// An ask put to an assistant is never the reader's.
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

/// What only text aimed at a model says (ADR 0009 Q4); a message holding any of it is asked
/// nothing.
/// Nothing here is a word ordinary mail uses.
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

/// "On Monday" says when, not by when.
pub(super) const DEADLINE_KEYWORDS: &[&str] = &["by", "before", "no later than"];

pub(super) const EVENT_OPENERS: &[&str] = &["the", "our", "your", "my", "this", "their", "next"];

pub(super) enum Idiom {
    Tonight,
    Friday,
    MonthEnd,
}

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

pub(super) const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "approx", "cf", "mr", "mrs", "ms", "dr",
];
