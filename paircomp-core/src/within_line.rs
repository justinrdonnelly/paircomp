use crate::file::open_regular_file;
use crate::{Error, Fingerprint, LineInfo};
use std::io::{self, Read};
use std::path::Path;

/// Inspects a 1-based line of a regular file.
///
/// Returns `Some` with its raw byte length, including any terminating LF, or
/// `None` beyond EOF. CR is ordinary content; a trailing LF does not create an
/// additional empty line. Invalid UTF-8 is accepted.
///
/// Reopens the path and requires [file stability](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::InvalidLineNumber`] for line zero, [`Error::Io`] if
/// metadata lookup, opening, or reading fails, or [`Error::NotRegularFile`]
/// for a non-regular input.
///
/// # Examples
///
/// See [`fingerprint_line_prefix`] for an example inspecting and hashing a line.
pub fn inspect_line(path: &Path, line: u64) -> Result<Option<LineInfo>, Error> {
    let byte_len = scan_line(open_regular_file(path)?, line, u64::MAX, |_| {})?;
    Ok((byte_len > 0).then_some(LineInfo { byte_len }))
}

/// Hashes only the first `byte_count` raw bytes of a 1-based line.
///
/// Zero bytes or an absent line hashes the empty sequence. Requests beyond
/// the line include its entire content, including LF if present, without
/// entering the next line or adding an EOF marker. Invalid UTF-8 is accepted.
/// The path must identify a readable regular file even when `byte_count` is
/// zero.
///
/// Reopens the path and requires [file stability](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::InvalidLineNumber`] for line zero, [`Error::Io`] if
/// metadata lookup, opening, or reading fails, or [`Error::NotRegularFile`]
/// for a non-regular input.
///
/// # Examples
///
/// A line-local prefix excludes preceding lines and may end inside a UTF-8
/// code point. Longer requests stop at the selected line's LF. Reference files
/// make the expected bytes explicit and are hashed with [`crate::fingerprint_file`].
///
/// ```
/// use paircomp_core::{
///     fingerprint_file, fingerprint_line_prefix, fingerprint_through_line, inspect_line,
/// };
/// use std::fs;
///
/// let directory = std::env::temp_dir()
///     .join(format!("paircomp-prefix-example-{}", std::process::id()));
/// fs::create_dir(&directory)?;
/// let path = directory.join("sample.txt");
/// fs::write(&path, "header\ncafé\nnext\n")?;
///
/// assert_eq!(inspect_line(&path, 2)?.map(|line| line.byte_len), Some(6));
/// let prefix = fingerprint_line_prefix(&path, 2, 4)?;
/// let prefix_path = directory.join("prefix.bin");
/// fs::write(&prefix_path, b"caf\xc3")?;
/// assert_eq!(prefix, fingerprint_file(&prefix_path)?);
///
/// let whole_line = fingerprint_line_prefix(&path, 2, 100)?;
/// let line_path = directory.join("line.txt");
/// fs::write(&line_path, "café\n")?;
/// assert_eq!(whole_line, fingerprint_file(&line_path)?);
///
/// // A file prefix also includes every preceding line.
/// let file_prefix = fingerprint_through_line(&path, 2)?;
/// let file_prefix_path = directory.join("file-prefix.txt");
/// fs::write(&file_prefix_path, "header\ncafé\n")?;
/// assert_eq!(file_prefix, fingerprint_file(&file_prefix_path)?);
///
/// // Zero bytes and an absent line both hash the empty sequence.
/// assert_eq!(fingerprint_line_prefix(&path, 2, 0)?, fingerprint_line_prefix(&path, 4, 100)?);
/// fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn fingerprint_line_prefix(
    path: &Path,
    line: u64,
    byte_count: u64,
) -> Result<Fingerprint, Error> {
    let mut hasher = blake3::Hasher::new();
    scan_line(open_regular_file(path)?, line, byte_count, |bytes| {
        hasher.update(bytes);
    })?;
    Ok(Fingerprint(*hasher.finalize().as_bytes()))
}

/// Maps a 1-based byte position to a 1-based Unicode code-point position.
///
/// Every byte of a multibyte character maps to the same character. Immediately
/// after an existing line, returns the next character position. CR and LF each
/// count as a code point; these positions are not visual editor columns.
///
/// Validates the entire selected line, returning `None` if any part is invalid
/// UTF-8, even after the requested byte. An absent line permits byte position
/// 1 and returns `None`. Other lines' encodings do not affect the result.
///
/// Reopens the path and requires [file stability](crate#file-stability).
///
/// # Errors
///
/// Returns [`Error::InvalidLineNumber`] for line zero and
/// [`Error::InvalidBytePosition`] for byte zero or a position more than one
/// past the line. Coordinate validation also applies to invalid UTF-8 lines.
/// Returns [`Error::Io`] if metadata lookup, opening, or reading fails,
/// [`Error::NotRegularFile`] for a non-regular input, or [`Error::FileTooLarge`]
/// if the next character position cannot fit in `u64`.
///
/// # Examples
///
/// Both bytes of `é` map to character 4. The terminating LF counts as a
/// separate code point, and the position immediately after it is also valid.
///
/// ```
/// use paircomp_core::utf8_character_position;
/// use std::fs;
///
/// let directory = std::env::temp_dir()
///     .join(format!("paircomp-utf8-example-{}", std::process::id()));
/// fs::create_dir(&directory)?;
/// let path = directory.join("sample.txt");
/// fs::write(&path, b"caf\xc3\xa9\nvalid prefix\xff\n")?;
///
/// assert_eq!(utf8_character_position(&path, 1, 4)?, Some(4));
/// assert_eq!(utf8_character_position(&path, 1, 5)?, Some(4));
/// assert_eq!(utf8_character_position(&path, 1, 6)?, Some(5)); // LF
/// assert_eq!(utf8_character_position(&path, 1, 7)?, Some(6)); // After LF
///
/// // Invalid UTF-8 later in line 2 suppresses even its first position.
/// assert_eq!(utf8_character_position(&path, 2, 1)?, None);
/// // A trailing LF does not create a third line.
/// assert_eq!(utf8_character_position(&path, 3, 1)?, None);
/// fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn utf8_character_position(path: &Path, line: u64, byte: u64) -> Result<Option<u64>, Error> {
    if byte == 0 {
        return Err(Error::InvalidBytePosition);
    }
    character_position(open_regular_file(path)?, line, byte)
}

/// Maps a byte position using a reader positioned at the start of the file.
///
/// The caller must reject byte zero before calling this helper.
fn character_position(reader: impl Read, line: u64, byte: u64) -> Result<Option<u64>, Error> {
    // Preserve an incomplete code point across chunks; UTF-8 needs at most four bytes.
    let mut pending = [0_u8; 4];
    let mut pending_len = 0;
    let mut valid = true;
    let mut seen = 0_u64;
    let mut characters = 0_u64;
    let mut position = None;
    let byte_len = scan_line(reader, line, u64::MAX, |bytes| {
        for &value in bytes {
            if !valid {
                // Keep scanning after invalid UTF-8 so the final byte bounds and
                // any later I/O errors are still checked.
                break;
            }
            seen += 1;
            pending[pending_len] = value;
            pending_len += 1;
            match std::str::from_utf8(&pending[..pending_len]) {
                Ok(_) => {
                    characters += 1;
                    if byte > seen - pending_len as u64 && byte <= seen {
                        position = Some(characters);
                    }
                    pending_len = 0;
                }
                Err(error) if error.error_len().is_none() && pending_len < 4 => {}
                Err(_) => valid = false,
            }
        }
    })?;
    // Subtraction also handles a hypothetical u64::MAX-byte line without overflow.
    if byte - 1 > byte_len {
        return Err(Error::InvalidBytePosition);
    }
    if !valid || pending_len != 0 || byte_len == 0 {
        return Ok(None);
    }
    if byte - 1 == byte_len {
        return Ok(Some(characters.checked_add(1).ok_or(Error::FileTooLarge)?));
    }
    Ok(position)
}

/// Visits bounded chunks from a single line, without retaining its contents.
///
/// The reader must start at the beginning of the file. Visits at most
/// `byte_limit` bytes, including the terminating LF if reached, and returns the
/// number visited. An absent line or a zero limit visits nothing and returns
/// zero. Line zero is invalid even when the limit is zero.
fn scan_line(
    mut reader: impl Read,
    line: u64,
    byte_limit: u64,
    mut visit: impl FnMut(&[u8]),
) -> Result<u64, Error> {
    if line == 0 {
        return Err(Error::InvalidLineNumber);
    }
    let mut lines_to_skip = line - 1;
    let mut byte_len = 0_u64;
    let mut buffer = [0_u8; 8192];
    while byte_len < byte_limit {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        let mut start = 0;
        while lines_to_skip > 0 && start < read {
            if buffer[start] == b'\n' {
                lines_to_skip -= 1;
            }
            start += 1;
        }
        if start == read {
            continue;
        }
        let bytes = &buffer[start..read];
        let line_end = bytes.iter().position(|&value| value == b'\n');
        let available = line_end.map_or(bytes.len(), |index| index + 1);
        let included = (available as u64).min(byte_limit - byte_len) as usize;
        byte_len = byte_len
            .checked_add(included as u64)
            .ok_or(Error::FileTooLarge)?;
        visit(&bytes[..included]);
        if line_end.is_some() {
            break;
        }
    }
    Ok(byte_len)
}

#[cfg(test)]
mod tests;
