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
use std::io;

mod file;
mod search;
pub use file::{fingerprint_file, fingerprint_through_line, inspect_file};
pub use search::{LineSearch, SearchStep};

/// Metadata calculated from the bytes read from a regular file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    /// Number of raw bytes in the file.
    pub byte_len: u64,
    /// Number of LF bytes, plus one for a nonempty unterminated suffix.
    ///
    /// A trailing LF creates no extra line; an empty file has zero lines.
    pub line_count: u64,
    /// BLAKE3 digest of all raw file bytes, without normalization.
    pub fingerprint: Fingerprint,
}

/// A full 256-bit BLAKE3 digest of raw bytes.
///
/// Equality compares all 32 digest bytes. Matching fingerprints provide strong
/// evidence of equal input, subject to the possibility of a hash collision.
/// Use [`Self::as_bytes`] to format the digest for display in a frontend.
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
