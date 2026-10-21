//! Reading a signature (specs/011-focus-contacts S9).
//!
//! A pure function from the words of a message to what its signature says
//! about the sender: a title, a company, phone numbers and websites. It reads
//! only the block at the tail of the message (after a `-- ` line, or after a
//! sign-off such as "Best,"), never the sentences above it, and never quoted
//! text. No I/O, no network: what is parsed stays on this machine, and the
//! caller decides what is kept.

use postio_model::SignatureDetails;

/// The words a signature is read by, from `data/signature.toml`.
#[derive(serde::Deserialize)]
struct Words {
    sign_offs: Vec<String>,
    title_words: Vec<String>,
    company_suffixes: Vec<String>,
}

fn words() -> &'static Words {
    static WORDS: std::sync::LazyLock<Words> = std::sync::LazyLock::new(|| {
        toml::from_str(include_str!("../data/signature.toml")).expect("the signature words parse")
    });
    &WORDS
}

/// What `text`'s signature says.
pub fn parse(text: &str) -> SignatureDetails {
    let block = signature_block(text);
    let mut details = SignatureDetails::default();
    // The name, when there is one, is the first line of the block.
    let mut first_line = true;
    for line in &block {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if details.name.is_none() && first_line && is_name(line) {
            details.name = Some(line.split_whitespace().collect::<Vec<_>>().join(" "));
            first_line = false;
            continue;
        }
        first_line = false;
        if let Some(phone) = phone_in(line) {
            if !details.phones.contains(&phone) {
                details.phones.push(phone);
            }
            continue;
        }
        if let Some(site) = website_in(line) {
            if !details.websites.contains(&site) {
                details.websites.push(site);
            }
            continue;
        }
        if details.title.is_none() || details.company.is_none() {
            work_in(line, &mut details);
        }
    }
    details
}

/// The lines of the signature: after a `-- ` line, else after the last
/// sign-off, up to eight of them, stopping at quoted text.
fn signature_block(text: &str) -> Vec<&str> {
    let all: Vec<&str> = text.lines().collect();
    // Quoted text and what follows it is not the sender's signature.
    let end = all
        .iter()
        .position(|line| line.trim_start().starts_with('>') || is_attribution(line))
        .unwrap_or(all.len());
    let all = &all[..end];
    let start = all
        .iter()
        .rposition(|line| line.trim_end() == "--" || line.trim_end() == "-- ")
        .map(|at| at + 1)
        .or_else(|| {
            all.iter()
                .rposition(|line| is_sign_off(line))
                // The sign-off, then the name: the name is the next line.
                .map(|at| at + 1)
        });
    match start {
        Some(start) => all[start..].iter().take(9).copied().collect(),
        None => Vec::new(),
    }
}

fn is_attribution(line: &str) -> bool {
    let line = line.trim();
    (line.starts_with("On ") && line.ends_with("wrote:")) || line.starts_with("-----Original")
}

fn is_sign_off(line: &str) -> bool {
    let line = line
        .trim()
        .trim_end_matches([',', '.', '!'])
        .to_ascii_lowercase();
    words().sign_offs.contains(&line)
}

/// A phone number in `line`: at least seven digits in a run of digits,
/// spaces, dots, dashes and parentheses, with an optional leading `+`.
fn phone_in(line: &str) -> Option<String> {
    let bytes: Vec<char> = line.chars().collect();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if c == '+' || c == '(' || c.is_ascii_digit() {
            let mut end = at;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || " .-()+".contains(bytes[end]))
            {
                end += 1;
            }
            let run: String = bytes[at..end].iter().collect();
            let run = run.trim().trim_end_matches(['.', '-']).trim().to_owned();
            let digits = run.chars().filter(char::is_ascii_digit).count();
            // A date (2026-09-26) is digits and dashes, and not a phone.
            let head: Vec<char> = run.chars().take(10).collect();
            let date_like = head.len() == 10
                && head.iter().enumerate().all(|(i, c)| match i {
                    4 | 7 => *c == '-',
                    _ => c.is_ascii_digit(),
                });
            if (7..=15).contains(&digits) && !date_like {
                return Some(run);
            }
            at = end.max(at + 1);
        } else {
            at += 1;
        }
    }
    None
}

/// A website in `line`: a `www.` or scheme address, or a bare `name.tld`
/// that is the whole line. Not an email address.
fn website_in(line: &str) -> Option<String> {
    let line = line.trim();
    if line.contains('@') {
        return None;
    }
    for word in line.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | ',' | ';'));
        let lower = word.to_ascii_lowercase();
        let stripped = lower
            .strip_prefix("https://")
            .or_else(|| lower.strip_prefix("http://"))
            .unwrap_or(&lower);
        let had_scheme = stripped.len() != lower.len();
        let stripped = stripped.strip_prefix("www.").unwrap_or(stripped);
        let bare_line = word == line;
        if (had_scheme || lower.starts_with("www.") || bare_line) && looks_like_domain(stripped) {
            return Some(stripped.trim_end_matches('/').to_owned());
        }
    }
    None
}

fn looks_like_domain(text: &str) -> bool {
    let host = text.split('/').next().unwrap_or("");
    let Some((name, tld)) = host.rsplit_once('.') else {
        return false;
    };
    !name.is_empty()
        && (2..=24).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A title and a company in `line`, in the shapes people write them.
fn work_in(line: &str, details: &mut SignatureDetails) {
    for separator in [" \u{b7} ", " | ", " \u{2022} ", " - ", " at ", ", "] {
        if let Some((left, right)) = line.split_once(separator) {
            let (left, right) = (left.trim(), right.trim());
            if left.is_empty() || right.is_empty() {
                continue;
            }
            if looks_like_title(left) || separator == " at " {
                if details.title.is_none() {
                    details.title = Some(left.to_owned());
                }
                if details.company.is_none() {
                    details.company = Some(right.to_owned());
                }
                return;
            }
            if looks_like_title(right) {
                if details.title.is_none() {
                    details.title = Some(right.to_owned());
                }
                if details.company.is_none() {
                    details.company = Some(left.to_owned());
                }
                return;
            }
        }
    }
    if details.company.is_none() && looks_like_company(line) {
        details.company = Some(line.to_owned());
    } else if details.title.is_none() && looks_like_title(line) {
        details.title = Some(line.to_owned());
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '.')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn looks_like_title(text: &str) -> bool {
    text.chars().count() <= 48
        && words_of(text)
            .iter()
            .any(|word| words().title_words.contains(word))
}

fn looks_like_company(text: &str) -> bool {
    text.chars().count() <= 48
        && words_of(text)
            .last()
            .is_some_and(|word| words().company_suffixes.contains(word))
}

/// A person's full name: two to four words, each starting with a capital and
/// made of letters, apostrophes, dots and hyphens.
fn is_name(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    (2..=4).contains(&words.len())
        && words.iter().all(|word| {
            word.chars().next().is_some_and(char::is_uppercase)
                && word
                    .chars()
                    .all(|c| c.is_alphabetic() || matches!(c, '\'' | '.' | '-' | '\u{2019}'))
        })
        && !looks_like_title(line)
        && !looks_like_company(line)
}
