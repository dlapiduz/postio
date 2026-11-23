//! One runner's share of a pass over the catalogue. `POSTIO_STORYBOARD_SHARD=2/4` takes the second
//! of four shares, round-robin, as `scripts/storyboards.sh run` deals its files to its shards.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shard {
    index: usize,
    count: usize,
}

impl Shard {
    pub const ALL: Self = Self { index: 1, count: 1 };

    pub fn parse(text: &str) -> Result<Self, String> {
        let refused = || format!("{text:?} is not k/n with 1 <= k <= n");
        let (index, count) = text.split_once('/').ok_or_else(refused)?;
        let index: usize = index.parse().map_err(|_| refused())?;
        let count: usize = count.parse().map_err(|_| refused())?;
        if index == 0 || index > count {
            return Err(refused());
        }
        Ok(Self { index, count })
    }

    /// `POSTIO_STORYBOARD_SHARD`, or the whole pass when it is unset. A value
    /// that does not parse is a panic, not the whole pass: a shard that
    /// quietly played everything would hide that the others play nothing new.
    pub fn from_env() -> Self {
        match std::env::var("POSTIO_STORYBOARD_SHARD") {
            Ok(text) => Self::parse(&text)
                .unwrap_or_else(|error| panic!("POSTIO_STORYBOARD_SHARD: {error}")),
            Err(_) => Self::ALL,
        }
    }

    pub fn takes(self, position: usize) -> bool {
        position % self.count == self.index - 1
    }

    pub fn count(self) -> usize {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::Shard;

    #[test]
    fn every_item_is_taken_by_exactly_one_shard() {
        let shards: Vec<Shard> = (1..=4)
            .map(|k| Shard::parse(&format!("{k}/4")).expect("parses"))
            .collect();
        for position in 0..50 {
            let takers = shards.iter().filter(|s| s.takes(position)).count();
            assert_eq!(takers, 1, "item {position} is taken {takers} times");
        }
    }

    #[test]
    fn the_whole_pass_takes_everything() {
        assert!((0..50).all(|position| Shard::ALL.takes(position)));
        assert_eq!(Shard::parse("1/1"), Ok(Shard::ALL));
    }

    #[test]
    fn shards_are_dealt_round_robin() {
        let second = Shard::parse("2/3").expect("parses");
        let taken: Vec<usize> = (0..9).filter(|&p| second.takes(p)).collect();
        assert_eq!(taken, [1, 4, 7]);
        assert_eq!(second.count(), 3);
    }

    #[test]
    fn a_shard_outside_its_count_is_refused() {
        for bad in ["0/4", "5/4", "1/0", "4", "a/b", "", "1/4/2"] {
            assert!(Shard::parse(bad).is_err(), "{bad:?} parsed");
        }
    }
}
