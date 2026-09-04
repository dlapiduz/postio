//! What a conversation looks like, decided without a toolkit.
//!
//! The conversation surfaces — the list row that stands for a thread, and the
//! pane that stacks its messages (ADR 0015) — draw the same facts twice, and
//! decided them twice: who is in a conversation was worked out inside a
//! `postio-gtk` widget, which made it unreachable from a second frontend and
//! provable only with a display.
//!
//! These are rules, not pixels, so they live here. What the widgets keep is
//! the drawing.

use postio_model::address::EmailAddress;

/// How many names fit before the line starts eliding.
const NAMES_SHOWN: usize = 3;

/// The people in a conversation, short and newest-biased.
///
/// Short names, because every surface that draws this gives it one line
/// beside something else: a subject and a snippet in the list, a message
/// count and a date span in the conversation header. A conversation between
/// four people spelled out in full would push the rest off. First name where
/// there is a display name, the address's local part where there is not —
/// the same information a person uses to recognise a thread at a glance.
///
/// **Newest-biased when it elides.** Participants arrive in first-seen order,
/// so the interesting end is the far one: who started the conversation still
/// identifies it, and who spoke most recently is what changed since you last
/// looked. Both survive; the middle is what goes.
pub fn participants(participants: &[EmailAddress]) -> String {
    // Distinct first, on the full name: one person writing five times is one
    // name, and two different people can share a first name.
    let mut distinct: Vec<&EmailAddress> = Vec::new();
    for address in participants {
        if !distinct
            .iter()
            .any(|seen| seen.display() == address.display())
        {
            distinct.push(address);
        }
    }

    // One participant is not a crowd, and shortening it loses information for
    // nothing: "Site Office" becomes "Site" and an address with no display
    // name becomes half of itself. A conversation with one voice reads
    // exactly like the message row it replaced, which is what the canvas
    // draws.
    if distinct.len() == 1 {
        return distinct[0].display().to_string();
    }

    let mut names: Vec<String> = Vec::new();
    for address in distinct {
        let name = short_name(address);
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    match names.len() {
        0 => String::new(),
        n if n <= NAMES_SHOWN => names.join(", "),
        _ => {
            let last = names.len();
            format!(
                "{} .. {}",
                names[0],
                names[last - (NAMES_SHOWN - 1)..].join(", ")
            )
        }
    }
}

/// One participant, shortened to the token that identifies them.
///
/// A display name is "Ada Norwood"; an address falls back to its local part,
/// which is what a sender without a name has to identify them. Trailing
/// punctuation goes with the quotes. "Bergstrom, Tove" is what Exchange and
/// most directories write, and its first token is `Bergstrom,` — which the
/// join above would then turn into "Bergstrom,, Jonas" (#826). Trimmed rather
/// than split on, because the comma is the only part that is noise: the
/// surname before it is exactly the short name this wants.
fn short_name(address: &EmailAddress) -> String {
    let display = address.display().to_string();
    let name = display
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| c == '"' || c == '\'')
        .trim_end_matches([',', ';'])
        .to_string();
    if name.contains('@') {
        return name.split('@').next().unwrap_or(&name).to_string();
    }
    if !name.is_empty() {
        return name;
    }
    // Nothing usable was left — a display name that is only punctuation, or
    // none at all. The local part is what identifies a sender who has not
    // given a name, which is the same answer the `@` branch above reaches
    // for; falling back to the raw display here would put the punctuation
    // straight back into the row.
    address
        .local_part()
        .map(str::to_owned)
        .unwrap_or_else(|| display.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- the participants line (ADR 0015 Q2, #307) ------------------------

    fn people(names: &[(&str, &str)]) -> Vec<EmailAddress> {
        names
            .iter()
            .map(|(name, address)| EmailAddress::new(Some(*name), *address))
            .collect()
    }

    #[test]
    fn one_participant_keeps_their_whole_name() {
        // A conversation with one voice has to read exactly like the message
        // row it replaced — every row in a folder is a thread row now, so
        // shortening here would shorten the ordinary case. "Site Office"
        // must not become "Site".
        assert_eq!(
            participants(&people(&[("Ada Norwood", "ada@example.com")])),
            "Ada Norwood"
        );
        assert_eq!(
            participants(&people(&[("Site Office", "site@example.com")])),
            "Site Office"
        );
    }

    #[test]
    fn a_last_first_display_name_does_not_leave_its_comma_behind() {
        // "Bergstrom, Tove" is what Exchange and most directories put in a
        // display name, and shortening it to the first whitespace token kept
        // the comma -- so joining the names produced "Bergstrom,, Jonas",
        // which reads as a rendering fault rather than as two people (#826).
        assert_eq!(
            participants(&people(&[
                ("Quinn Abara", "quinn@example.net"),
                ("Bergstrom, Tove", "tove@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "Quinn, Bergstrom, Jonas"
        );
    }

    #[test]
    fn a_last_first_name_survives_the_elision_too() {
        // The elided branch joins with ", " as well, so it has the same
        // problem and needs the same proof -- this is the shape the shot in
        // #826 actually showed.
        assert_eq!(
            participants(&people(&[
                ("Quinn Abara", "quinn@example.net"),
                ("Ada Norwood", "ada@example.com"),
                ("Priya Raman", "priya@example.org"),
                ("Bergstrom, Tove", "tove@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "Quinn .. Bergstrom, Jonas"
        );
    }

    #[test]
    fn a_trailing_comma_is_not_mistaken_for_the_whole_name() {
        // The trim must not eat a name that is only punctuation, or a sender
        // whose display name is a stray comma would vanish from the row
        // rather than being named by their address.
        assert_eq!(
            participants(&people(&[
                (",", "odd@example.com"),
                ("Jonas Vek", "jonas@example.org"),
            ])),
            "odd, Jonas"
        );
    }

    #[test]
    fn a_short_conversation_names_everyone_in_it() {
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Tove Bergstrom", "tove@example.com"),
            ])),
            "Ada, Quinn, Tove"
        );
    }

    #[test]
    fn a_long_conversation_keeps_both_ends_and_drops_the_middle() {
        // Newest-biased: whoever started it still identifies the
        // conversation, and whoever spoke last is what changed since you
        // looked. The middle is what nobody scans for.
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
                ("Jonas Vek", "jonas@example.org"),
                ("Tove Bergstrom", "tove@example.com"),
                ("Priya Raman", "priya@example.org"),
            ])),
            "Ada .. Tove, Priya"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_is_named_once() {
        assert_eq!(
            participants(&people(&[
                ("Ada Norwood", "ada@example.com"),
                ("Ada Norwood", "ada@example.com"),
                ("Quinn Abara", "quinn@example.net"),
            ])),
            "Ada, Quinn"
        );
    }

    #[test]
    fn one_person_writing_repeatedly_and_alone_is_not_a_crowd() {
        // Deduplication happens before the "is this one voice" test, or a
        // thread where one person replied to themselves would read as two.
        assert_eq!(
            participants(&people(&[
                ("Site Office", "site@example.com"),
                ("Site Office", "site@example.com"),
            ])),
            "Site Office"
        );
    }

    #[test]
    fn a_lone_sender_with_no_display_name_keeps_their_whole_address() {
        assert_eq!(
            participants(&[EmailAddress::new(None::<&str>, "ada.norwood@example.com")]),
            "ada.norwood@example.com"
        );
    }

    #[test]
    fn a_crowd_with_no_display_names_is_named_by_local_parts() {
        // Several addresses do not fit; their local parts do, and are what
        // distinguishes them.
        assert_eq!(
            participants(&[
                EmailAddress::new(None::<&str>, "ada@example.com"),
                EmailAddress::new(None::<&str>, "quinn@example.net"),
            ]),
            "ada, quinn"
        );
    }

    #[test]
    fn no_participants_is_empty_rather_than_a_placeholder() {
        // A message row has none, and `initials_source` falls back to the
        // sender rather than drawing something that looks like a thread.
        assert_eq!(participants(&[]), "");
    }
}
