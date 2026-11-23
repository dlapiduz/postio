//! The automated-senders table: who sends mail no person wrote, as data (constitution VII).
//! Embedded `data/senders.toml`, generic forms only, never a provider's domain.

mod table;

use std::fmt;
use std::sync::LazyLock;

use postio_model::EmailAddress;

use crate::outcome::{ReasonKind, SourceName};

/// `build.rs` validated this same file with the same module, so the `expect` cannot fire in a build
/// that succeeded.
static SHIPPED: LazyLock<Senders> = LazyLock::new(|| {
    Senders::parse(include_str!("../../data/senders.toml"))
        .expect("build.rs validated data/senders.toml")
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Senders {
    entries: Vec<Sender>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    name: String,
    local: Vec<Vec<String>>,
    domain: Vec<String>,
    // None: a machine's mail, often the one thing to act on, so it is not filed.
    reason: Option<ReasonKind>,
    source: Option<SourceName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendersError(table::TableError);

impl fmt::Display for SendersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SendersError {}

impl Senders {
    pub fn shipped() -> &'static Senders {
        &SHIPPED
    }

    pub fn parse(text: &str) -> Result<Senders, SendersError> {
        let rows = table::parse(text).map_err(SendersError)?;
        let entries = rows
            .into_iter()
            .map(|row| Sender {
                reason: row.reason.as_deref().map(reason),
                source: row.source.map(SourceName),
                name: row.name,
                local: row.local,
                domain: row.domain,
            })
            .collect();
        Ok(Senders { entries })
    }

    pub fn find(&self, address: &EmailAddress) -> Option<&Sender> {
        let (local, domain) = address.address.rsplit_once('@')?;
        let words: Vec<String> = local
            .split(table::SEPARATORS)
            .map(str::to_ascii_lowercase)
            .collect();
        let labels: Vec<String> = domain.split('.').map(str::to_ascii_lowercase).collect();
        let subdomains = &labels[..labels.len().saturating_sub(2)];
        self.entries.iter().find(|sender| {
            sender.local.iter().any(|pattern| {
                words
                    .windows(pattern.len())
                    .any(|run| run == pattern.as_slice())
            }) || sender.domain.iter().any(|label| subdomains.contains(label))
        })
    }
}

fn reason(spelled: &str) -> ReasonKind {
    ReasonKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == spelled)
        .expect("table::parse accepts only the vocabulary's spellings")
}

impl Sender {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn reason(&self) -> Option<ReasonKind> {
        self.reason
    }

    pub fn source(&self) -> Option<&SourceName> {
        self.source.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn found(address: &str) -> Option<&'static str> {
        Senders::shipped()
            .find(&EmailAddress::new(None::<&str>, address))
            .map(Sender::name)
    }

    #[test]
    fn the_shipped_patterns_match_their_fixtures() {
        for (address, entry) in [
            ("noreply@example.com", "no-reply"),
            ("no-reply@accounts.example.org", "no-reply"),
            ("No_Reply@example.net", "no-reply"),
            ("no.reply@example.test", "no-reply"),
            ("donotreply@example.com", "no-reply"),
            ("do-not-reply@example.com", "no-reply"),
            ("billing-noreply@example.com", "no-reply"),
            ("noreply+bounce-7@example.com", "no-reply"),
            ("notifications@forge.example", "notifications"),
            ("notification@calendar.example", "notifications"),
            ("notify@example.org", "notifications"),
            ("alerts@monitoring.example", "notifications"),
            ("security-alert@bank.example", "notifications"),
            ("hello@notifications.example.com", "notification-subdomain"),
            ("team@mail.notify.example.org", "notification-subdomain"),
            ("receipts@shop.example", "receipts"),
            ("receipt@cafe.example", "receipts"),
            ("tracking@parcels.example", "shipping"),
            ("shipment-updates@parcels.example", "shipping"),
            ("offers@shop.example", "marketing"),
            ("promotions@shop.example", "marketing"),
            ("deals@travel.example", "marketing"),
        ] {
            assert_eq!(found(address), Some(entry), "{address}");
        }
    }

    #[test]
    fn a_person_and_a_mailbox_people_answer_are_not_automated() {
        for address in [
            "ada.norwood@example.com",
            // A pattern's words, not its letters: "noah" is not "no".
            "noah@example.com",
            "valerta@example.org",
            "renotify.team@example.net",
            // People answer these, and a bounce is news, not an update.
            "support@example.com",
            "billing@example.com",
            "info@example.com",
            "mailer-daemon@example.com",
            // A domain of two labels is a registrable name, not a
            // subdomain.
            "someone@notifications.example",
        ] {
            assert_eq!(found(address), None, "{address}");
        }
    }

    #[test]
    fn a_match_says_why_and_who() {
        let sender = Senders::shipped()
            .find(&EmailAddress::new(
                Some("Forge"),
                "notifications@forge.example",
            ))
            .expect("a notifier");

        assert_eq!(sender.reason(), Some(ReasonKind::Notification));
        assert_eq!(sender.source(), None, "no shipped entry names a source");
    }

    #[test]
    fn the_first_entry_that_matches_wins() {
        let table = Senders::parse(
            "[[sender]]\nname = \"first\"\nlocal = [\"offers\"]\nreason = \"promotion\"\n\
             source = \"Offers\"\n\
             [[sender]]\nname = \"second\"\nlocal = [\"offers\"]\nreason = \"spam\"\n",
        )
        .expect("a table");

        let sender = table
            .find(&EmailAddress::new(None::<&str>, "offers@shop.example"))
            .expect("a match");
        assert_eq!(sender.name(), "first");
        assert_eq!(sender.reason(), Some(ReasonKind::Promotion));
        assert_eq!(sender.source().map(SourceName::as_str), Some("Offers"));
    }

    #[test]
    fn a_machine_that_writes_for_a_company_is_found_with_no_reason() {
        for (address, entry) in [
            ("statements@email.bank.example", "bulk-subdomain"),
            ("hello@news.utility.example", "bulk-subdomain"),
            ("billing@e.power.example", "bulk-subdomain"),
            ("automated@utility.example", "machine"),
        ] {
            let sender = Senders::shipped()
                .find(&EmailAddress::new(None::<&str>, address))
                .unwrap_or_else(|| panic!("{address} is a machine"));
            assert_eq!(sender.name(), entry, "{address}");
            assert_eq!(sender.reason(), None, "{address} is filed by nothing here");
        }
        for person in [
            "tove@mail.example.org",
            "tove@email.example",
            "emily@example.com",
            "news.desk@paper.example",
        ] {
            assert_eq!(found(person), None, "{person}");
        }
    }

    fn load_error(text: &str) -> String {
        Senders::parse(text)
            .expect_err("the table does not load")
            .to_string()
    }

    #[test]
    fn a_bad_entry_is_a_load_error_that_names_it() {
        for (text, entry) in [
            (
                "[[sender]]\nname = \"marketing\"\nlocal = [\"offers\"]\nreason = \"promo\"\n",
                "`marketing`",
            ),
            (
                "[[sender]]\nname = \"empty\"\nreason = \"notification\"\n",
                "`empty`",
            ),
            (
                "[[sender]]\nname = \"address\"\nlocal = [\"noreply@example.com\"]\n\
                 reason = \"notification\"\n",
                "`address`",
            ),
            (
                "[[sender]]\nname = \"dotted\"\ndomain = [\"notifications.example\"]\n\
                 reason = \"notification\"\n",
                "`dotted`",
            ),
            (
                "[[sender]]\nname = \"blank\"\nlocal = [\"\"]\nreason = \"notification\"\n",
                "`blank`",
            ),
            (
                "[[sender]]\nname = \"typo\"\nlocals = [\"noreply\"]\nreason = \"notification\"\n",
                "`typo`",
            ),
            (
                "[[sender]]\nname = \"twice\"\nlocal = [\"a\"]\nreason = \"spam\"\n\
                 [[sender]]\nname = \"twice\"\nlocal = [\"b\"]\nreason = \"spam\"\n",
                "`twice`",
            ),
            (
                "[[sender]]\nname = \"ok\"\nlocal = [\"a\"]\nreason = \"spam\"\n\
                 [[sender]]\nlocal = [\"b\"]\nreason = \"spam\"\n",
                "#2",
            ),
            (
                "[[sender]]\nname = \"named\"\nlocal = [\"a\"]\nsource = \"A\"\n",
                "`named`",
            ),
        ] {
            let error = load_error(text);
            assert!(
                error.contains(&format!("entry {entry}")),
                "{error:?} names {entry}"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_table_is_a_load_error() {
        assert!(load_error("[[sender]\nname =").starts_with("senders table: "));
        assert!(load_error("sender = 3\n").starts_with("senders table: "));
    }

    #[test]
    fn an_empty_table_loads_and_knows_nobody() {
        let table = Senders::parse("# nothing yet\n").expect("an empty table");

        assert_eq!(table, Senders::default());
        assert!(
            table
                .find(&EmailAddress::new(None::<&str>, "noreply@example.com"))
                .is_none()
        );
    }

    #[test]
    fn the_table_s_reasons_are_the_vocabulary_s() {
        // The parser spells the vocabulary itself, because build.rs compiles
        // it with nothing else of the crate; this holds the two together.
        assert_eq!(table::REASONS, ReasonKind::ALL.map(ReasonKind::as_str));
    }

    #[test]
    fn a_provider_in_code_would_be_found() {
        let patterns = vec!["noreply".to_owned()];
        let source = "fn f(from: &str) -> bool {\n\
                      // noreply@forge.example is only a comment\n\
                      let table = include_str!(\"../data/senders.toml\");\n\
                      from.ends_with(\"@forge.example.com\") || from == \"noreply\"\n\
                      }\n\
                      #[cfg(test)]\n\
                      mod tests {\n\
                      const A: &str = \"ada@example.com\";\n\
                      }\n";

        let found = provider_constants(source, &patterns);

        assert_eq!(
            found,
            [
                "line 4: \"@forge.example.com\"".to_owned(),
                "line 4: \"noreply\"".to_owned(),
            ]
        );
    }

    #[test]
    fn the_classifier_s_source_holds_no_provider_constant() {
        // Every pattern the classifier matches comes from data/senders.toml; its source spells none
        // (constitution VII).
        let patterns: Vec<String> = Senders::shipped()
            .entries
            .iter()
            .flat_map(|sender| {
                sender
                    .local
                    .iter()
                    .map(|words| words.join("-"))
                    .chain(sender.domain.iter().cloned())
            })
            // A verb the detector reads ("email me the form") is a word, not a provider.
            .filter(|pattern| !crate::needs_action::IMPERATIVES.contains(&pattern.as_str()))
            .collect();
        assert!(!patterns.is_empty(), "the shipped table has patterns");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        let mut found = Vec::new();
        let mut read = 0;
        for file in sources(&root) {
            let source = std::fs::read_to_string(&file).expect("a source file");
            read += literals(&code_of(&source)).len();
            for finding in provider_constants(&source, &patterns) {
                found.push(format!("{}: {finding}", file.display()));
            }
        }

        assert!(found.is_empty(), "{}", found.join("\n"));
        assert!(read > 300, "the scan read only {read} literals");
    }

    fn sources(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(root).expect("a directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                out.extend(sources(&path));
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
        out
    }

    /// Comments are prose, `include_str!`'s path is a file, and the reasons' spellings are the
    /// store's vocabulary, which two of the table's patterns happen to share.
    fn provider_constants(source: &str, patterns: &[String]) -> Vec<String> {
        literals(&code_of(source))
            .into_iter()
            .filter(|(_, literal, included)| {
                !included
                    && (literal.contains('@')
                        || hostname(literal)
                        || (patterns.iter().any(|p| literal.eq_ignore_ascii_case(p))
                            && !table::REASONS.contains(&literal.as_str())))
            })
            .map(|(line, literal, _)| format!("line {line}: {literal:?}"))
            .collect()
    }

    fn code_of(source: &str) -> String {
        let mut code = String::new();
        let mut lines = source.lines().peekable();
        while let Some(line) = lines.next() {
            let test_module = line.trim() == "#[cfg(test)]"
                && lines
                    .peek()
                    .is_some_and(|next| next.trim_start().starts_with("mod "));
            if test_module {
                break;
            }
            code.push_str(line);
            code.push('\n');
        }
        code
    }

    fn literals(code: &str) -> Vec<(usize, String, bool)> {
        let chars: Vec<char> = code.chars().collect();
        let mut out = Vec::new();
        let (mut at, mut line) = (0, 1);
        while at < chars.len() {
            match chars[at] {
                '\n' => line += 1,
                '/' if chars.get(at + 1) == Some(&'/') => {
                    while at < chars.len() && chars[at] != '\n' {
                        at += 1;
                    }
                    continue;
                }
                '/' if chars.get(at + 1) == Some(&'*') => {
                    at += 2;
                    while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                        line += usize::from(chars[at] == '\n');
                        at += 1;
                    }
                    at += 1;
                }
                '\'' if chars.get(at + 1) == Some(&'\\') => {
                    let mut escaped = at + 2;
                    if chars.get(escaped) == Some(&'u') {
                        while escaped < chars.len() && chars[escaped] != '}' {
                            escaped += 1;
                        }
                    }
                    at = escaped + 1;
                }
                '\'' if chars.get(at + 2) == Some(&'\'') => at += 2,
                'r' if matches!(chars.get(at + 1), Some('"' | '#')) => {
                    let hashes = chars[at + 1..].iter().take_while(|c| **c == '#').count();
                    let open = at + 1 + hashes;
                    if chars.get(open) == Some(&'"') {
                        let closing: String = std::iter::once('"')
                            .chain("#".repeat(hashes).chars())
                            .collect();
                        let rest: String = chars[open + 1..].iter().collect();
                        let length = rest.find(&closing).unwrap_or(rest.len());
                        let text: String = rest.chars().take(length).collect();
                        out.push((line, text.clone(), included(&chars[..at])));
                        line += text.matches('\n').count();
                        at = open + 1 + text.chars().count() + closing.chars().count();
                        continue;
                    }
                }
                '"' => {
                    let start = at;
                    let mut text = String::new();
                    at += 1;
                    while at < chars.len() && chars[at] != '"' {
                        if chars[at] == '\\' {
                            at += 1;
                        }
                        text.push(chars[at]);
                        at += 1;
                    }
                    out.push((line, text.clone(), included(&chars[..start])));
                    line += text.matches('\n').count();
                }
                _ => {}
            }
            at += 1;
        }
        out
    }

    fn included(before: &[char]) -> bool {
        let before: String = before.iter().collect();
        let before = before.trim_end();
        before.ends_with("include_str!(") || before.ends_with("include_bytes!(")
    }

    fn hostname(literal: &str) -> bool {
        literal
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .any(|run| {
                let labels: Vec<&str> = run.split('.').collect();
                let last = labels.last().copied().unwrap_or("");
                labels.len() >= 2
                    && labels.iter().all(|label| !label.is_empty())
                    && last.len() >= 2
                    && last.chars().all(|c| c.is_ascii_alphabetic())
                    && !["rs", "toml", "md", "txt"].contains(&last)
            })
    }
}
