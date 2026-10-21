//! Scoring two records that may be one person (specs/011-focus-contacts S10).
//!
//! A pure function from what is known of two records to the evidence that
//! they are the same person, as lines a person can read. No evidence, no
//! candidate: the sheet shows nothing the scorer did not say. A colleague at
//! the same company is not evidence; a shared name, the same title and company
//! in the signature, an address that stopped when another began in the same
//! threads, a `Resent-From` header, and answers that crossed are.

/// What is known of one record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Side {
    /// The name shown.
    pub name: String,
    /// Every address the record owns.
    pub addresses: Vec<String>,
    /// The title their signature gives.
    pub title: Option<String>,
    /// The company their signature gives.
    pub company: Option<String>,
}

/// What is known of the two together, from the mail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pair {
    /// In how many threads one stopped writing as the other began.
    pub took_over_in_threads: u32,
    /// The address that stopped, and the address that began, when it did.
    pub took_over: Option<(String, String)>,
    /// The address a `Resent-From` header named as the one the other
    /// forwards from, when one did.
    pub forwards_from: Option<String>,
    /// The address that forwards, when `forwards_from` is set.
    pub forwards_to: Option<String>,
    /// How many answers crossed between them.
    pub replies: u32,
    /// A phone number both give.
    pub shared_phone: Option<String>,
    /// One contact card an import read that lists both.
    pub same_card: Option<String>,
    /// The title and company both sign with, when the signatures were noted
    /// as the same.
    pub signature: Option<(String, String)>,
}

/// One reason to think two records are one person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// Both are called this.
    SameName(String),
    /// The same title and company in the signature.
    SameSignature {
        /// The title.
        title: String,
        /// The company.
        company: String,
    },
    /// One address stopped when the other began, in the same threads.
    TookOver {
        /// The address that stopped.
        old: String,
        /// The address that began.
        new: String,
        /// How many threads.
        threads: u32,
    },
    /// A `Resent-From` header.
    Forwards {
        /// Where it forwards from.
        from: String,
        /// The address that forwards.
        to: String,
    },
    /// Both give the same phone number.
    SamePhone(String),
    /// One contact card an import read lists both.
    SameCard,
    /// Mail sent to one was answered from the other.
    Replied {
        /// How many answers.
        answers: u32,
    },
}

impl Evidence {
    /// How much it counts for.
    pub fn weight(&self) -> u32 {
        match self {
            Self::SameName(_) => 2,
            Self::SameSignature { .. } => 4,
            Self::TookOver { threads, .. } => 3 + (*threads).min(5) / 2,
            Self::Forwards { .. } => 4,
            Self::SamePhone(_) => 4,
            Self::SameCard => 5,
            Self::Replied { answers } => 2 + (*answers).min(4) / 2,
        }
    }
}

/// The score of a pair that has evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    /// The sum of the evidence's weights.
    pub score: u32,
    /// What there is.
    pub evidence: Vec<Evidence>,
}

fn folded(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The evidence that `a` and `b` are one person, or `None` when there is none.
pub fn score(a: &Side, b: &Side, pair: &Pair) -> Option<Scored> {
    let mut evidence = Vec::new();
    let (fa, fb) = (folded(&a.name), folded(&b.name));
    if !fa.is_empty() && fa == fb && !a.name.contains('@') {
        let shown = a.name.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown = if a.name.trim() == shown && b.name.trim() == shown {
            shown
        } else {
            a.name.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        evidence.push(Evidence::SameName(shown));
    }
    if let (Some(ta), Some(ca), Some(tb), Some(cb)) = (&a.title, &a.company, &b.title, &b.company)
        && folded(ta) == folded(tb)
        && folded(ca) == folded(cb)
    {
        evidence.push(Evidence::SameSignature {
            title: ta.clone(),
            company: ca.clone(),
        });
    }
    if let Some((title, company)) = &pair.signature
        && !evidence
            .iter()
            .any(|e| matches!(e, Evidence::SameSignature { .. }))
    {
        evidence.push(Evidence::SameSignature {
            title: title.clone(),
            company: company.clone(),
        });
    }
    if pair.took_over_in_threads >= 2
        && let Some((old, new)) = &pair.took_over
    {
        evidence.push(Evidence::TookOver {
            old: old.clone(),
            new: new.clone(),
            threads: pair.took_over_in_threads,
        });
    } else if pair.took_over_in_threads >= 2 {
        evidence.push(Evidence::TookOver {
            old: b.addresses.first().cloned().unwrap_or_default(),
            new: a.addresses.first().cloned().unwrap_or_default(),
            threads: pair.took_over_in_threads,
        });
    }
    if let Some(from) = &pair.forwards_from {
        evidence.push(Evidence::Forwards {
            from: from.clone(),
            to: pair
                .forwards_to
                .clone()
                .or_else(|| b.addresses.first().cloned())
                .unwrap_or_default(),
        });
    }
    if pair.same_card.is_some() {
        evidence.push(Evidence::SameCard);
    }
    if let Some(phone) = &pair.shared_phone {
        evidence.push(Evidence::SamePhone(phone.clone()));
    }
    if pair.replies > 0 {
        evidence.push(Evidence::Replied {
            answers: pair.replies,
        });
    }
    if evidence.is_empty() {
        return None;
    }
    Some(Scored {
        score: evidence.iter().map(Evidence::weight).sum(),
        evidence,
    })
}

/// The sheet's "Why we think so" lines for a group of `size` records, from
/// each pair's evidence. A kind of evidence every pair shares is said once for
/// the group; the rest are said as they are. `name` is the kept record's.
pub fn describe(size: usize, pairs: &[Vec<Evidence>], name: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let shared = |matches: &dyn Fn(&Evidence) -> bool| {
        !pairs.is_empty() && pairs.iter().all(|pair| pair.iter().any(matches))
    };
    let everyone = match size {
        0..=2 => "both".to_owned(),
        3 => "all three".to_owned(),
        4 => "all four".to_owned(),
        n => format!("all {n}"),
    };
    let first = |matches: &dyn Fn(&Evidence) -> bool| -> Option<&Evidence> {
        pairs.iter().flatten().find(|e| matches(e))
    };
    if shared(&|e| matches!(e, Evidence::SameName(_)))
        && let Some(Evidence::SameName(shown)) = first(&|e| matches!(e, Evidence::SameName(_)))
    {
        lines.push(format!(
            "{everyone} are called \u{201c}{shown}\u{201d}",
            everyone = capital(&everyone)
        ));
    }
    if shared(&|e| matches!(e, Evidence::SameSignature { .. }))
        && let Some(Evidence::SameSignature { title, company }) =
            first(&|e| matches!(e, Evidence::SameSignature { .. }))
    {
        let on = if size == 2 {
            "both".to_owned()
        } else {
            everyone.clone()
        };
        lines.push(format!(
            "The same signature on {on}: \u{201c}{name} \u{b7} {title}, {company}\u{201d}"
        ));
    }
    for evidence in pairs.iter().flatten() {
        if let Evidence::SamePhone(phone) = evidence {
            let line = format!("{} give the phone number {phone}", capital(&everyone));
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    if pairs
        .iter()
        .flatten()
        .any(|evidence| matches!(evidence, Evidence::SameCard))
    {
        let on = if size == 2 {
            "both".to_owned()
        } else {
            everyone.clone()
        };
        lines.push(format!("One contact card you imported lists {on}"));
    }
    let mut seen_took = Vec::new();
    let mut seen_forwards = Vec::new();
    for evidence in pairs.iter().flatten() {
        match evidence {
            Evidence::TookOver { old, new, threads } if !seen_took.contains(&(old, new)) => {
                seen_took.push((old, new));
                lines.push(format!(
                    "{old} stopped when {new} began, in the same {threads} threads"
                ));
            }
            Evidence::Forwards { from, to } if !seen_forwards.contains(&(from, to)) => {
                seen_forwards.push((from, to));
                lines.push(format!("{to} forwards from {from} (Resent-From header)"));
            }
            Evidence::Replied { answers } => {
                let line = format!(
                    "Mail to one was answered from the other, {answers} time{}",
                    if *answers == 1 { "" } else { "s" }
                );
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
            _ => {}
        }
    }
    lines
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// What a message's body and headers note about its sender, for the review to
/// find look-alikes by: the signature (when it gives a name, a title and a
/// company), each phone number, and the address a `Resent-From` header names.
/// Facts about the sender, never message content.
///
/// With `read_signatures` false the signature is not parsed at all
/// (Settings > Contacts, "Read signatures for details"): only the forwarding
/// header is looked at.
pub fn noticed(
    text: &str,
    headers: &str,
    read_signatures: bool,
) -> Vec<(postio_model::EvidenceKind, String)> {
    use postio_model::EvidenceKind;
    let mut found = Vec::new();
    if read_signatures {
        let signature = crate::signature::parse(text);
        if let Some(key) = signature.key() {
            found.push((EvidenceKind::Signature, key));
        }
        for phone in &signature.phones {
            found.push((
                EvidenceKind::Phone,
                phone.split_whitespace().collect::<Vec<_>>().join(" "),
            ));
        }
    }
    if let Some(address) = resent_from(headers) {
        found.push((EvidenceKind::Resent, address));
    }
    found
}

/// The first address of a `Resent-From` header, lowercased.
fn resent_from(headers: &str) -> Option<String> {
    let mut lines = headers.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("resent-from") {
            continue;
        }
        let mut value = value.to_owned();
        while lines
            .peek()
            .is_some_and(|next| next.starts_with(' ') || next.starts_with('\t'))
        {
            value.push(' ');
            value.push_str(lines.next()?.trim());
        }
        return postio_model::address::parse_list(&value)
            .into_iter()
            .find(|address| address.is_plausible())
            .map(|address| address.address.to_ascii_lowercase());
    }
    None
}
