//! A verdict's moment: when it was decided, never when it was published
//! (`github-apps`, Verdict publication). Milliseconds in UTC, written at a
//! fixed width so the text orders as the instants do.

use jiff::Timestamp;

/// When a verdict was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Moment(i64);

impl Moment {
    /// The moment `at` falls in.
    pub fn at(at: Timestamp) -> Self {
        Self(at.as_millisecond())
    }

    /// The first moment after this one.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The moment a verdict decided at `at` over one of moment `over` has:
    /// `at`, or the first moment after `over` where `at` is not later.
    pub fn decided(at: Timestamp, over: Option<Moment>) -> Self {
        let moment = Self::at(at);
        match over {
            Some(over) if moment <= over => over.next(),
            _ => moment,
        }
    }

    /// The fixed-width text `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    pub fn text(self) -> String {
        let millis = self.0.rem_euclid(1000);
        let at = Timestamp::from_millisecond(self.0 - millis).unwrap_or(Timestamp::UNIX_EPOCH);
        format!("{}.{millis:03}Z", at.strftime("%Y-%m-%dT%H:%M:%S"))
    }

    /// The moment `text` writes; `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        let parsed = text.parse::<Timestamp>().ok().map(Self::at)?;
        (parsed.text() == text).then_some(parsed)
    }
}

impl std::fmt::Display for Moment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_recorded_no_later_than_its_predecessor_takes_the_first_moment_after_it() {
        let earlier: Timestamp = "2026-11-24T10:00:00.500Z".parse().unwrap();
        let predecessor = Moment::at("2026-11-24T10:00:01Z".parse().unwrap());
        assert_eq!(
            Moment::decided(earlier, Some(predecessor)),
            predecessor.next()
        );
    }

    #[test]
    fn a_moment_written_at_its_fixed_width_reads_back_as_itself() {
        let moment = Moment::at("2026-11-24T10:00:01.007Z".parse().unwrap());
        assert_eq!(Moment::parse(&moment.text()), Some(moment));
    }
}
