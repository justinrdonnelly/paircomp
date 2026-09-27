use crate::{Error, FileInfo, Fingerprint};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

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

/// Fingerprints the file prefix through the selected line, including any LF.
///
/// Hashes from the beginning of the file, rather than just the selected line.
/// Line numbers start at 1; line zero hashes the empty sequence. Requests beyond
/// EOF hash all available bytes. CR is ordinary content; no normalization or
/// UTF-8 decoding occurs.
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
            for (index, &byte) in buffer[..read].iter().enumerate() {
                if byte == b'\n' {
                    lf_count = lf_count.checked_add(1).ok_or(Error::FileTooLarge)?;
                    if through_line == Some(lf_count) {
                        // The terminating LF belongs to the requested prefix.
                        included = index + 1;
                        reached_line = true;
                        break;
                    }
                }
            }

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
