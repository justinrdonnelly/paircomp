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
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(f),
            Self::Io(error) => write!(f, "terminal I/O failed: {error}"),
            Self::Aborted => f.write_str("input ended before the comparison was complete"),
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
    let mut diagnostics = io::stderr().lock();
    match run(
        &cli.file,
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut diagnostics,
    ) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(diagnostics, "paircomp: {error}");
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
    diagnostics: &mut impl Write,
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
        diagnostics,
        "Does this fingerprint match the other copy? [y/n] ",
    )? {
        writeln!(output, "Files match.")?;
        return Ok(ExitCode::SUCCESS);
    }

    let other_line_count = read_count(
        input,
        output,
        diagnostics,
        "Line count displayed by the other copy: ",
        "line",
    )?;
    let mut search = LineSearch::new(info.line_count, other_line_count)?;
    loop {
        match search.current_step() {
            LineSearchStep::CompareThroughLine { line } => {
                let fingerprint = fingerprint_through_line(path, line)?;
                writeln!(output)?;
                writeln!(output, "Compare through line {line}:")?;
                writeln!(output, "Fingerprint: {}", format_fingerprint(fingerprint))?;
                let matched = read_match(
                    input,
                    output,
                    diagnostics,
                    "Does this fingerprint match? [y/n] ",
                )?;
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
                if read_yes_no(
                    input,
                    output,
                    diagnostics,
                    "Continue within this line? [Y/n] ",
                    Some(true),
                )? {
                    localize_byte(path, line, input, output, diagnostics)?;
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
    diagnostics: &mut impl Write,
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
        diagnostics,
        "Byte count displayed for this line by the other copy: ",
        "byte",
    )?;
    let mut search = ByteSearch::new(local_byte_len, other_byte_len)?;
    loop {
        match search.current_step() {
            ByteSearchStep::CompareThroughByte { byte } => {
                let fingerprint = fingerprint_line_prefix(path, line, byte)?;
                writeln!(output)?;
                writeln!(output, "Compare line {line} through byte {byte}:")?;
                writeln!(output, "Fingerprint: {}", format_fingerprint(fingerprint))?;
                let matched = read_match(
                    input,
                    output,
                    diagnostics,
                    "Does this fingerprint match? [y/n] ",
                )?;
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
    diagnostics: &mut impl Write,
    prompt: &str,
) -> Result<bool, WorkflowError> {
    read_yes_no(input, output, diagnostics, prompt, None)
}

/// Reads a trimmed, case-insensitive yes/no answer with an optional default.
///
/// A submitted blank answer is invalid without a default. The caller's prompt
/// must show `[y/n]` without a default, or capitalize the default choice.
/// Invalid answers repeat the prompt; EOF and I/O failures abort.
fn read_yes_no(
    input: &mut impl BufRead,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
    prompt: &str,
    default: Option<bool>,
) -> Result<bool, WorkflowError> {
    loop {
        let answer = read_prompt(input, output, prompt)?;
        let answer = answer.trim();
        if answer.is_empty() {
            if let Some(default) = default {
                return Ok(default);
            }
        } else if answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
            return Ok(false);
        } else if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            return Ok(true);
        }
        writeln!(
            diagnostics,
            "paircomp: invalid answer; enter y, yes, n, or no"
        )?;
    }
}

/// Reads a decimal `u64`, rejecting blank input, signs, and overflow.
///
/// Surrounding whitespace is ignored. Malformed input repeats the prompt with a
/// diagnostic naming the count kind; EOF and I/O failures abort.
fn read_count(
    input: &mut impl BufRead,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
    prompt: &str,
    kind: &str,
) -> Result<u64, WorkflowError> {
    loop {
        let answer = read_prompt(input, output, prompt)?;
        let answer = answer.trim();
        if !answer.is_empty() && answer.bytes().all(|byte| byte.is_ascii_digit()) {
            if let Ok(count) = answer.parse() {
                return Ok(count);
            }
        }
        writeln!(
            diagnostics,
            "paircomp: invalid {kind} count; enter a nonnegative decimal u64"
        )?;
    }
}
