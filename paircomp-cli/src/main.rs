use clap::Parser;
use paircomp_core::{
    fingerprint_line_prefix, fingerprint_through_line, inspect_file, inspect_line,
    utf8_character_position, ByteSearch, ByteSearchStep, Fingerprint, LineSearch, LineSearchStep,
};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "paircomp",
    version,
    about = "Compare isolated file copies using fingerprints"
)]
struct Cli {
    /// Local file to compare
    file: PathBuf,
}

#[derive(Debug)]
enum WorkflowError {
    Core(paircomp_core::Error),
    Io(io::Error),
    Aborted,
    InvalidAnswer,
    InvalidLineCount,
    InvalidByteCount,
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(f),
            Self::Io(error) => write!(f, "terminal I/O failed: {error}"),
            Self::Aborted => f.write_str("input ended before the comparison was complete"),
            Self::InvalidAnswer => f.write_str("invalid answer; enter y, yes, n, or no"),
            Self::InvalidLineCount => {
                f.write_str("invalid line count; enter a nonnegative decimal u64")
            }
            Self::InvalidByteCount => {
                f.write_str("invalid byte count; enter a nonnegative decimal u64")
            }
        }
    }
}

impl From<paircomp_core::Error> for WorkflowError {
    fn from(error: paircomp_core::Error) -> Self {
        Self::Core(error)
    }
}

impl From<io::Error> for WorkflowError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli.file, &mut io::stdin().lock(), &mut io::stdout().lock()) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "paircomp: {error}");
            ExitCode::from(2)
        }
    }
}

/// Drives a comparison session, returning status 0 for a match or 1 for a difference.
///
/// Failed or aborted interactions return an error for `main` to report with status 2.
fn run(
    path: &Path,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<ExitCode, WorkflowError> {
    let info = inspect_file(path)?;
    writeln!(output, "File: {}", path.display())?;
    writeln!(output, "Lines: {}", info.line_count)?;
    writeln!(output, "Size: {} bytes", info.byte_len)?;
    writeln!(
        output,
        "Fingerprint: {}",
        format_fingerprint(info.fingerprint)
    )?;
    writeln!(output)?;
    writeln!(
        output,
        "Keep both files unchanged during this session. Restart after editing either file."
    )?;
    writeln!(output)?;

    if read_match(
        input,
        output,
        "Does this fingerprint match the other copy? [y/N] ",
    )? {
        writeln!(output, "Files match.")?;
        return Ok(ExitCode::SUCCESS);
    }

    let other_line_count = read_count(
        input,
        output,
        "Line count displayed by the other copy: ",
        WorkflowError::InvalidLineCount,
    )?;
    let mut search = LineSearch::new(info.line_count, other_line_count)?;
    loop {
        match search.current_step() {
            LineSearchStep::CompareThroughLine { line } => {
                let fingerprint = fingerprint_through_line(path, line)?;
                writeln!(output)?;
                writeln!(output, "Compare through line {line}:")?;
                writeln!(output, "Fingerprint: {}", format_fingerprint(fingerprint))?;
                let matched = read_match(input, output, "Does this fingerprint match? [y/N] ")?;
                search.record_result(matched)?;
            }
            LineSearchStep::DifferenceAtLine { line } => {
                writeln!(output)?;
                writeln!(output, "First divergence: line {line}")?;
                if line > info.line_count {
                    writeln!(output, "The local file ends before line {line}.")?;
                }
                writeln!(
                    output,
                    "Choose the same continuation answer on both copies."
                )?;
                if read_yes_no(input, output, "Continue within this line? [Y/n] ", true)? {
                    localize_byte(path, line, input, output)?;
                }
                writeln!(output)?;
                writeln!(
                    output,
                    "Inspect/correct the corresponding files, then run paircomp again."
                )?;
                return Ok(ExitCode::from(1));
            }
        }
    }
}

/// Drives the optional byte search within the first differing line.
///
/// The line search must already have established that preceding lines match.
fn localize_byte(
    path: &Path,
    line: u64,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), WorkflowError> {
    let local = inspect_line(path, line)?;
    let local_byte_len = local.map_or(0, |info| info.byte_len);
    writeln!(output)?;
    writeln!(
        output,
        "Line {line} size: {local_byte_len} bytes (including any CR/LF)"
    )?;
    let other_byte_len = read_count(
        input,
        output,
        "Byte count displayed for this line by the other copy: ",
        WorkflowError::InvalidByteCount,
    )?;
    let mut search = ByteSearch::new(local_byte_len, other_byte_len)?;
    loop {
        match search.current_step() {
            ByteSearchStep::CompareThroughByte { byte } => {
                let fingerprint = fingerprint_line_prefix(path, line, byte)?;
                writeln!(output)?;
                writeln!(output, "Compare line {line} through byte {byte}:")?;
                writeln!(output, "Fingerprint: {}", format_fingerprint(fingerprint))?;
                let matched = read_match(input, output, "Does this fingerprint match? [y/N] ")?;
                search.record_result(matched)?;
            }
            ByteSearchStep::DifferenceAtByte { byte } => {
                let character = utf8_character_position(path, line, byte)?;
                writeln!(output)?;
                write!(output, "First divergence: line {line}, byte {byte}")?;
                if let Some(character) = character {
                    write!(output, " (UTF-8 character {character})")?;
                }
                writeln!(output)?;
                if local.is_none() {
                    writeln!(
                        output,
                        "The local file ends before line {line}; this byte is absent."
                    )?;
                } else if byte > local_byte_len {
                    writeln!(
                        output,
                        "The local line ends before byte {byte}; this byte is absent."
                    )?;
                }
                return Ok(());
            }
        }
    }
}

/// Formats the full digest as 64 lowercase hexadecimal digits without truncation.
fn format_fingerprint(fingerprint: Fingerprint) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut formatted = String::with_capacity(64);
    for &byte in fingerprint.as_bytes() {
        formatted.push(char::from(HEX[usize::from(byte >> 4)]));
        formatted.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    formatted
}

/// Flushes a prompt and reads an answer, retaining its terminating newline.
///
/// Only a newline submits an answer. EOF, including after partial input, aborts
/// the session so that it cannot silently select a prompt's default.
fn read_prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> Result<String, WorkflowError> {
    write!(output, "{prompt}")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !answer.ends_with('\n') {
        return Err(WorkflowError::Aborted);
    }
    Ok(answer)
}

fn read_match(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> Result<bool, WorkflowError> {
    read_yes_no(input, output, prompt, false)
}

/// Reads a trimmed, case-insensitive yes/no answer with a caller-selected default.
///
/// The default applies only to a submitted blank answer; the caller's prompt
/// must display the corresponding `[y/N]` or `[Y/n]` choice.
fn read_yes_no(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
    default: bool,
) -> Result<bool, WorkflowError> {
    let answer = read_prompt(input, output, prompt)?;
    let answer = answer.trim();
    if answer.is_empty() {
        Ok(default)
    } else if answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
        Ok(false)
    } else if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
        Ok(true)
    } else {
        Err(WorkflowError::InvalidAnswer)
    }
}

/// Reads a decimal `u64`, rejecting blank input, signs, and overflow.
///
/// Surrounding whitespace is ignored. Malformed input returns `invalid`, allowing
/// the caller to distinguish line-count and byte-count diagnostics.
fn read_count(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
    invalid: WorkflowError,
) -> Result<u64, WorkflowError> {
    let answer = read_prompt(input, output, prompt)?;
    let answer = answer.trim();
    if answer.is_empty() || !answer.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid);
    }
    answer.parse().map_err(|_| invalid)
}
