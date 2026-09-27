use crate::Error;

/// A prefix comparison requested by [`LineSearch`], or its completed result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineSearchStep {
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
    /// use paircomp_core::{LineSearch, LineSearchStep};
    ///
    /// // The whole-file fingerprints differ; the copies have 3 and 4 lines.
    /// let mut search = LineSearch::new(3, 4)?;
    /// assert_eq!(search.current_step(), LineSearchStep::CompareThroughLine { line: 2 });
    /// search.record_result(true)?; // The prefixes through line 2 match.
    /// assert_eq!(search.current_step(), LineSearchStep::CompareThroughLine { line: 3 });
    /// search.record_result(false)?; // The prefixes through line 3 differ.
    /// assert_eq!(search.current_step(), LineSearchStep::DifferenceAtLine { line: 3 });
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
    /// candidate remains, returns [`LineSearchStep::DifferenceAtLine`] without
    /// requesting another comparison.
    pub fn current_step(&self) -> LineSearchStep {
        match self.bounds.midpoint() {
            Some(line) => LineSearchStep::CompareThroughLine { line },
            None => LineSearchStep::DifferenceAtLine {
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

/// A requested line-local prefix comparison, or the first differing byte.
///
/// Byte positions are 1-based and can denote a byte absent from the shorter copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteSearchStep {
    /// Compare prefixes from the selected line's beginning through this byte.
    ///
    /// Use [`crate::fingerprint_line_prefix`] on each copy. Requests past the
    /// local line's end hash the whole line; an absent line hashes no bytes.
    CompareThroughByte {
        /// The 1-based byte position.
        byte: u64,
    },
    /// The first differing byte; it may be absent from the shorter line.
    DifferenceAtByte {
        /// The 1-based byte position.
        byte: u64,
    },
}

/// A deterministic search for the first differing byte within a differing line.
///
/// Supply both copies' byte lengths, including any LF, to [`Self::new`]. An
/// absent line has length zero. Drive the search with [`Self::current_step`] and
/// [`Self::record_result`]; the caller keeps track of the selected line, and the
/// search itself performs no file I/O.
///
/// Counts and comparison results must describe the same unchanged pair of lines
/// throughout the search. See the [file stability requirements](crate#file-stability).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteSearch {
    bounds: SearchBounds,
}

impl ByteSearch {
    /// Creates a byte search using both copies' counts.
    ///
    /// Construct only after the line search has established that the selected
    /// line differs and all preceding lines match. Supply each copy's byte length
    /// for that line, including any CR/LF; use zero for an absent line. Both files
    /// must stay unchanged, and both instances must use accurate lengths and the
    /// same comparison answers.
    ///
    /// The larger count determines the shared upper bound. The constructor does
    /// not read either file or verify the reported mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyLinesCannotDiffer`] when both counts are zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use paircomp_core::{ByteSearch, ByteSearchStep};
    ///
    /// // The differing lines are UTF-8 "café" and "cafè", each 5 bytes long.
    /// let mut search = ByteSearch::new(5, 5)?;
    /// assert_eq!(search.current_step(), ByteSearchStep::CompareThroughByte { byte: 3 });
    /// search.record_result(true)?; // "caf" matches.
    /// assert_eq!(search.current_step(), ByteSearchStep::CompareThroughByte { byte: 4 });
    /// search.record_result(true)?; // The first byte of the final code point matches.
    /// assert_eq!(search.current_step(), ByteSearchStep::DifferenceAtByte { byte: 5 });
    /// # Ok::<(), paircomp_core::Error>(())
    /// ```
    pub fn new(local_byte_len: u64, other_byte_len: u64) -> Result<Self, Error> {
        let bounds = SearchBounds::new(local_byte_len.max(other_byte_len))
            .ok_or(Error::EmptyLinesCannotDiffer)?;
        Ok(Self { bounds })
    }

    /// Returns the next prefix comparison or the completed byte result.
    ///
    /// Repeated calls leave the state unchanged. Positions are 1-based; a final
    /// result can refer to a byte absent from the shorter copy. When only one
    /// candidate remains, returns [`ByteSearchStep::DifferenceAtByte`] without
    /// requesting another comparison.
    pub fn current_step(&self) -> ByteSearchStep {
        match self.bounds.midpoint() {
            Some(byte) => ByteSearchStep::CompareThroughByte { byte },
            None => ByteSearchStep::DifferenceAtByte {
                byte: self.bounds.low,
            },
        }
    }

    /// Applies the answer to the comparison returned by [`Self::current_step`].
    ///
    /// Pass `true` when the two fingerprints from [`crate::fingerprint_line_prefix`]
    /// match, or `false` when they differ. A match excludes the prefix through the
    /// requested byte; a mismatch retains that position as a candidate. The
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
