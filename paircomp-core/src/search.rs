use crate::Error;

/// A prefix comparison requested by [`LineSearch`], or its completed result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchStep {
    /// Compare file prefixes through this line, including its LF if present.
    ///
    /// Use [`crate::fingerprint_through_line`] on each copy. A request beyond
    /// the local EOF hashes the whole local file.
    CompareThroughLine {
        /// The 1-based line position.
        line: u64,
    },
    /// The first differing line; it may be beyond the local EOF.
    DifferenceAtLine {
        /// The 1-based line position.
        line: u64,
    },
}

/// A deterministic search for the first differing line using file prefixes.
///
/// Construct this only after the whole-file fingerprints have been reported
/// as different. Supply the local line count and the count reported by the
/// other instance to [`Self::new`]. Drive the search with [`Self::current_step`]
/// and [`Self::record_result`]; the search itself performs no file I/O.
///
/// Line counts and comparison results must describe the same unchanged pair of
/// files throughout the search. See the [file stability requirements](crate#file-stability).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineSearch {
    bounds: SearchBounds,
}

impl LineSearch {
    /// Creates a line search using both copies' counts.
    ///
    /// Construct only after comparing the whole-file fingerprints and establishing
    /// a mismatch. Supply the local line count and the count reported by the other
    /// copy; either count may be zero. Both files must stay unchanged, and both
    /// instances must use accurate counts and the same comparison answers.
    ///
    /// The larger count determines the shared upper bound. The constructor does
    /// not read either file or verify the reported mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyFilesCannotDiffer`] when both counts are zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use paircomp_core::{LineSearch, SearchStep};
    ///
    /// // The whole-file fingerprints differ; the copies have 3 and 4 lines.
    /// let mut search = LineSearch::new(3, 4)?;
    /// assert_eq!(search.current_step(), SearchStep::CompareThroughLine { line: 2 });
    /// search.record_result(true)?; // The prefixes through line 2 match.
    /// assert_eq!(search.current_step(), SearchStep::CompareThroughLine { line: 3 });
    /// search.record_result(false)?; // The prefixes through line 3 differ.
    /// assert_eq!(search.current_step(), SearchStep::DifferenceAtLine { line: 3 });
    /// # Ok::<(), paircomp_core::Error>(())
    /// ```
    pub fn new(local_line_count: u64, other_line_count: u64) -> Result<Self, Error> {
        let bounds = SearchBounds::new(local_line_count.max(other_line_count))
            .ok_or(Error::EmptyFilesCannotDiffer)?;
        Ok(Self { bounds })
    }

    /// Returns the next prefix comparison or the completed line result.
    ///
    /// Repeated calls leave the state unchanged. Positions are 1-based; a final
    /// result can refer to a line absent from the shorter copy. When only one
    /// candidate remains, returns [`SearchStep::DifferenceAtLine`] without
    /// requesting another comparison.
    pub fn current_step(&self) -> SearchStep {
        match self.bounds.midpoint() {
            Some(line) => SearchStep::CompareThroughLine { line },
            None => SearchStep::DifferenceAtLine {
                line: self.bounds.low,
            },
        }
    }

    /// Applies the answer to the comparison returned by [`Self::current_step`].
    ///
    /// Pass `true` when the two fingerprints from [`crate::fingerprint_through_line`]
    /// match, or `false` when they differ. A match excludes the prefix through the
    /// requested line; a mismatch retains that position as a candidate. The
    /// caller must supply the answer for the current comparison on both copies.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SearchAlreadyComplete`] if the result is already known.
    /// An error leaves the search unchanged.
    pub fn record_result(&mut self, matched: bool) -> Result<(), Error> {
        self.bounds.record_result(matched)
    }
}

/// Inclusive candidate bounds for prefix bisection.
///
/// Maintains `1 <= low <= high`. Given an initial mismatch and consistent
/// answers, prefixes before `low` match and the prefix through `high` differs.
/// This lets the search finish at `low == high` without another comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SearchBounds {
    low: u64,
    high: u64,
}

impl SearchBounds {
    fn new(high: u64) -> Option<Self> {
        (high > 0).then_some(Self { low: 1, high })
    }

    /// A completed search has no midpoint; `low` is its result.
    fn midpoint(&self) -> Option<u64> {
        (self.low < self.high).then(|| self.low + (self.high - self.low) / 2)
    }

    fn record_result(&mut self, matched: bool) -> Result<(), Error> {
        let midpoint = self.midpoint().ok_or(Error::SearchAlreadyComplete)?;
        if matched {
            self.low = midpoint + 1;
        } else {
            self.high = midpoint;
        }
        Ok(())
    }
}
