//! Raw-byte file inspection, prefix fingerprinting, and difference localization.
//!
//! Each instance reads only its local file. The caller exchanges counts and
//! compares fingerprints with the other instance; this crate handles file
//! access and search state without terminal or network I/O.
//!
//! # Comparison workflow
//!
//! 1. Call [`inspect_file`] and compare the whole-file fingerprints. If they
//!    match, the comparison is complete.
//! 2. On a mismatch, exchange line counts and construct a [`LineSearch`]. Use
//!    [`fingerprint_through_line`] for each requested comparison and pass the
//!    answer to [`LineSearch::record_result`] until the differing line is known.
//! 3. To continue within that line, call [`inspect_line`], exchange byte lengths
//!    (zero for an absent line), and construct a [`ByteSearch`]. Compare
//!    [`fingerprint_line_prefix`] results and call [`ByteSearch::record_result`].
//! 4. Optionally annotate the differing byte with [`utf8_character_position`].
//!
//! Both instances must use accurate counts and the same comparison answers.
//! Localization also assumes that distinct prefixes have distinct fingerprints.
//!
//! # Bytes and positions
//!
//! Hashing uses raw bytes without normalization or UTF-8 decoding. LF terminates
//! a line and belongs to it; CR is ordinary content. A nonempty suffix after the
//! last LF counts as a line, and a trailing LF creates no extra line.
//!
//! Line, byte, and Unicode code-point positions start at 1. Only
//! [`fingerprint_through_line`] accepts line zero, denoting an empty file prefix.
//! Byte positions are relative to the selected line. Character annotations
//! require that entire line to be valid UTF-8 and count code points, not grapheme
//! clusters or visual columns.
//!
//! # File stability
//!
//! Each file operation opens the supplied path afresh. The library does
//! not snapshot files, lock them, or detect changes.
//!
//! Both files must remain unchanged from the start of the initial inspection
//! until the comparison ends, including during reads and between calls. If
//! either file is edited or replaced, discard the collected metadata,
//! fingerprints, and [`LineSearch`]/[`ByteSearch`] state, and restart both
//! instances from the initial inspection.

use std::error::Error as StdError;
use std::fmt;
use std::io;

mod file;
mod search;
mod within_line;
pub use file::{fingerprint_file, fingerprint_through_line, inspect_file};
pub use search::{ByteSearch, ByteSearchStep, LineSearch, LineSearchStep};
pub use within_line::{fingerprint_line_prefix, inspect_line, utf8_character_position};

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

/// Metadata for an existing line, including its terminating LF if present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineInfo {
    /// Number of raw bytes in the line, including its LF if present.
    ///
    /// An existing line always contains at least one byte.
    pub byte_len: u64,
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
    /// A byte count, line count, or character position cannot fit in `u64`.
    FileTooLarge,
    /// Both supplied line counts are zero despite a reported mismatch.
    EmptyFilesCannotDiffer,
    /// Both supplied line byte lengths are zero despite a reported mismatch.
    EmptyLinesCannotDiffer,
    /// A line-local operation was given line zero; its lines start at 1.
    InvalidLineNumber,
    /// The byte position is zero or more than one past the selected line.
    ///
    /// An absent line permits only byte position 1.
    InvalidBytePosition,
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
            Self::EmptyLinesCannotDiffer => f.write_str("two zero-length lines cannot differ"),
            Self::InvalidLineNumber => f.write_str("line numbers must start at 1"),
            Self::InvalidBytePosition => {
                f.write_str("byte position must be within the line or immediately after it")
            }
            Self::SearchAlreadyComplete => f.write_str("search is already complete"),
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
            | Self::EmptyLinesCannotDiffer
            | Self::InvalidLineNumber
            | Self::InvalidBytePosition
            | Self::SearchAlreadyComplete => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
