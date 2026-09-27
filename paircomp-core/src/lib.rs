//! Raw-byte file inspection, prefix fingerprinting, and line search for Paircomp.
//!
//! # File stability
//!
//! Each call to [`inspect_file`], [`fingerprint_file`], or
//! [`fingerprint_through_line`] opens the supplied path afresh. The library does
//! not snapshot files, lock them, or detect changes.
//!
//! Both files must remain unchanged from the start of the initial inspection
//! until the comparison ends, including during reads and between calls. If
//! either file is edited or replaced, discard the collected metadata,
//! fingerprints, and [`LineSearch`] state, and restart both instances from the
//! initial inspection.

use std::error::Error as StdError;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Metadata calculated from the bytes read from a regular file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    /// Number of raw bytes in the file.
    pub byte_len: u64,
    /// Number of LF bytes, plus one for a nonempty unterminated suffix.
    /// A trailing LF creates no extra line; an empty file has zero lines.
    pub line_count: u64,
    /// BLAKE3 digest of all raw file bytes, without normalization.
    pub fingerprint: Fingerprint,
}

/// A full 256-bit BLAKE3 digest. Presentation is left to the frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Borrows the complete 32-byte BLAKE3 digest in its original byte order.
    ///
    /// No bytes are truncated or converted to a display string.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// An error returned by a core operation.
#[derive(Debug)]
pub enum Error {
    /// Metadata lookup, opening, or reading the file failed.
    Io(io::Error),
    /// The path does not identify a regular file.
    NotRegularFile,
    /// A byte count or line count cannot fit in `u64`.
    FileTooLarge,
    /// Both supplied line counts are zero despite a reported mismatch.
    EmptyFilesCannotDiffer,
    /// An answer was submitted after the search had reached its result.
    SearchAlreadyComplete,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "file I/O failed: {error}"),
            Self::NotRegularFile => f.write_str("input is not a regular file"),
            Self::FileTooLarge => f.write_str("file size or line count exceeds u64"),
            Self::EmptyFilesCannotDiffer => f.write_str("two empty files cannot differ"),
            Self::SearchAlreadyComplete => f.write_str("line search is already complete"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::NotRegularFile
            | Self::FileTooLarge
            | Self::EmptyFilesCannotDiffer
            | Self::SearchAlreadyComplete => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The comparison requested by a line search, or its typed result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchStep {
    /// Compare file prefixes through this line, including its LF if present.
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

/// Inclusive candidate bounds for the first differing line.
///
/// Construct this only after the whole-file fingerprints have been reported
/// as different. The caller supplies each instance's line count and the
/// count reported by the other instance.
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

/// Inspects a regular file without interpreting its bytes as text.
///
/// Returns its raw byte length, line count, and full-file BLAKE3 fingerprint.
/// Lines end at LF, which belongs to the line; a nonempty unterminated suffix
/// also counts as a line. A trailing LF adds no extra line. Empty files have
/// zero bytes and lines. CR is ordinary content, and invalid UTF-8 is accepted.
///
/// Each call reopens the path. Keep the file unchanged throughout inspection
/// and any subsequent comparison; restart after editing or replacing it.
/// See the [file stability requirements](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::Io`] if metadata lookup, opening, or reading fails,
/// [`Error::NotRegularFile`] for a non-regular input, or
/// [`Error::FileTooLarge`] if a byte or line count cannot fit in `u64`.
///
/// # Examples
///
/// ```no_run
/// use paircomp_core::inspect_file;
/// use std::path::Path;
///
/// let info = inspect_file(Path::new("local-copy.txt"))?;
/// println!("{} lines, {} bytes", info.line_count, info.byte_len);
/// // Display the full digest for manual comparison with the other system.
/// print!("Fingerprint: ");
/// for byte in info.fingerprint.as_bytes() {
///     print!("{byte:02x}");
/// }
/// println!();
/// # Ok::<(), paircomp_core::Error>(())
/// ```
pub fn inspect_file(path: &Path) -> Result<FileInfo, Error> {
    scan_file(path, None)
}

/// Fingerprints every raw byte of a regular file using BLAKE3.
///
/// No whitespace, newline, or encoding normalization is performed. Empty
/// files and arbitrary non-UTF-8 bytes are accepted. Returns the same digest
/// as [`inspect_file`], whose example shows how to access the digest bytes.
///
/// Each call reopens the path. Keep the file unchanged throughout inspection
/// and any subsequent comparison; restart after editing or replacing it.
/// See the [file stability requirements](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::Io`] if metadata lookup, opening, or reading fails,
/// [`Error::NotRegularFile`] for a non-regular input, or
/// [`Error::FileTooLarge`] if a byte or line count cannot fit in `u64`.
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint, Error> {
    Ok(inspect_file(path)?.fingerprint)
}

/// Fingerprints the file prefix through a 1-based line, including its LF.
///
/// Hashes from the beginning of the file, rather than just the selected line.
/// Line zero hashes the empty sequence. Requests beyond EOF hash all available
/// bytes. CR is ordinary content; no normalization or UTF-8 decoding occurs.
/// The path must identify a readable regular file even when `line` is zero.
///
/// Each call reopens the path. Keep the file unchanged throughout inspection
/// and any subsequent comparison; restart after editing or replacing it.
/// See the [file stability requirements](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::Io`] if metadata lookup, opening, or reading fails,
/// [`Error::NotRegularFile`] for a non-regular input, or
/// [`Error::FileTooLarge`] if a byte or line count cannot fit in `u64`.
pub fn fingerprint_through_line(path: &Path, line: u64) -> Result<Fingerprint, Error> {
    Ok(scan_file(path, Some(line))?.fingerprint)
}

fn scan_file(path: &Path, through_line: Option<u64>) -> Result<FileInfo, Error> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(Error::NotRegularFile);
    }
    scan_reader(File::open(path)?, through_line)
}

fn scan_reader(mut reader: impl Read, through_line: Option<u64>) -> Result<FileInfo, Error> {
    let mut hasher = blake3::Hasher::new();
    let mut byte_len = 0_u64;
    let mut lf_count = 0_u64;
    let mut ends_with_lf = false;
    let mut buffer = [0_u8; 8192];

    if through_line != Some(0) {
        loop {
            // `read` is the number of bytes this call put into the buffer.
            let read = match reader.read(&mut buffer) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if read == 0 {
                break;
            }

            // Use the whole chunk unless the requested line ends inside it.
            let mut included = read;
            let mut reached_line = false;
            // Count LF bytes across chunks; each LF ends one line.
            for (index, &byte) in buffer[..read].iter().enumerate() {
                if byte == b'\n' {
                    lf_count = lf_count.checked_add(1).ok_or(Error::FileTooLarge)?;
                    if through_line == Some(lf_count) {
                        // Include this LF. `index + 1` is never greater than `read`.
                        included = index + 1;
                        reached_line = true;
                        break;
                    }
                }
            }

            // This borrows the bytes to hash; it does not fill a new array.
            let bytes = &buffer[..included];
            hasher.update(bytes);
            byte_len = byte_len
                .checked_add(u64::try_from(included).map_err(|_| Error::FileTooLarge)?)
                .ok_or(Error::FileTooLarge)?;
            ends_with_lf = bytes.last() == Some(&b'\n');

            if reached_line {
                break;
            }
        }
    }

    let line_count = lf_count
        .checked_add(u64::from(byte_len > 0 && !ends_with_lf))
        .ok_or(Error::FileTooLarge)?;
    Ok(FileInfo {
        byte_len,
        line_count,
        fingerprint: Fingerprint(*hasher.finalize().as_bytes()),
    })
}

#[cfg(test)]
mod tests;
