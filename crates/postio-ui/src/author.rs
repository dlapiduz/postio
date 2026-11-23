use postio_model::thread_view::ThreadMember;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    You,
    Named(String),
    Someone,
}

impl Author {
    pub fn of(member: &ThreadMember) -> Self {
        match (&member.from, member.own) {
            (_, true) => Author::You,
            (Some(from), false) => Author::Named(crate::focus_row::first_name(from).to_owned()),
            (None, false) => Author::Someone,
        }
    }

    pub fn subject(&self) -> &str {
        match self {
            Author::You => "You",
            Author::Named(name) => name,
            Author::Someone => "Someone",
        }
    }

    pub fn mid_sentence(&self) -> &str {
        match self {
            Author::You => "you",
            Author::Named(name) => name,
            Author::Someone => "someone",
        }
    }

    pub fn possessive(&self) -> String {
        match self {
            Author::You => "your".to_owned(),
            Author::Named(name) => format!("{name}\u{2019}s"),
            Author::Someone => "someone\u{2019}s".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_reads_in_its_place_in_a_sentence() {
        let forms = [
            (Author::You, "You", "you", "your"),
            (Author::Named("Ben".into()), "Ben", "Ben", "Ben\u{2019}s"),
            (Author::Someone, "Someone", "someone", "someone\u{2019}s"),
        ];
        for (author, subject, mid, possessive) in forms {
            assert_eq!(author.subject(), subject);
            assert_eq!(author.mid_sentence(), mid);
            assert_eq!(author.possessive(), possessive);
        }
    }
}
