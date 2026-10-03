mod presentation;

use clap::{ColorChoice, Parser};
use paircomp_core::{
    fingerprint_line_prefix, fingerprint_through_line, inspect_file, inspect_line,
    utf8_character_position, ByteSearch, ByteSearchStep, Fingerprint, LineSearch, LineSearchStep,
};
use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use presentation::{color_for_stream, Palette, Presentation};

#[derive(Parser)]
#[command(
    name = "paircomp",
    version,
    about = "Compare isolated file copies using fingerprints"
)]
struct Cli {
    /// Local file to compare
    file: PathBuf,

    /// Display all 64 hexadecimal digits instead of the first 8
    #[arg(long)]
    full_digest: bool,

    /// Control color and emphasis in comparison output
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
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
    let presentation = Presentation {
        output: Palette::new(color_for_stream(cli.color, io::stdout().is_terminal())),
        diagnostics: Palette::new(color_for_stream(cli.color, io::stderr().is_terminal())),
    };
    let mut diagnostics = io::stderr().lock();
    match run(
        &cli.file,
        cli.full_digest,
        presentation,
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut diagnostics,
    ) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(
                diagnostics,
                "{} {error}",
                presentation.diagnostics.error("paircomp:")
            );
            ExitCode::from(2)
        }
    }
}

/// Drives a comparison session, returning status 0 for a match or 1 for a difference.
///
/// Failed or aborted interactions return an error for `main` to report with status 2.
fn run(
    path: &Path,
    full_digest: bool,
    presentation: Presentation,
    input: &mut impl BufRead,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<ExitCode, WorkflowError> {
    let palette = presentation.output;
    let info = inspect_file(path)?;
    writeln!(output, "File: {}", path.display())?;
    writeln!(output, "Lines: {}", palette.bold(info.line_count))?;
    writeln!(output, "Size: {} bytes", palette.bold(info.byte_len))?;
    writeln!(
        output,
        "Fingerprint: {}",
        palette.bold(format_fingerprint(info.fingerprint, full_digest))
    )?;
    writeln!(output)?;
    writeln!(
        output,
        "Use the same fingerprint display mode on both copies."
    )?;
    writeln!(
        output,
        "Keep both files unchanged during this session. Restart after editing either file."
    )?;
    writeln!(output)?;

    if read_match(
        input,
        output,
        diagnostics,
        presentation,
        "Does this fingerprint match the other copy? [y/n] ",
    )? {
        writeln!(output, "{}", palette.matched("Files match."))?;
        return Ok(ExitCode::SUCCESS);
    }

    let other_line_count = read_count(
        input,
        output,
        diagnostics,
        presentation,
        "Enter the other copy's line count: ",
        "line",
    )?;
    let mut search = LineSearch::new(info.line_count, other_line_count)?;
    loop {
        match search.current_step() {
            LineSearchStep::CompareThroughLine { line } => {
                let fingerprint = fingerprint_through_line(path, line)?;
                writeln!(output)?;
                writeln!(
                    output,
                    "{}",
                    palette.heading(format_args!("Compare through line {line}:"))
                )?;
                writeln!(
                    output,
                    "Fingerprint: {}",
                    palette.bold(format_fingerprint(fingerprint, full_digest))
                )?;
                let matched = read_match(
                    input,
                    output,
                    diagnostics,
                    presentation,
                    "Does this fingerprint match? [y/n] ",
                )?;
                search.record_result(matched)?;
            }
            LineSearchStep::DifferenceAtLine { line } => {
                writeln!(output)?;
                writeln!(
                    output,
                    "{}",
                    palette.difference(format_args!("First divergence: line {line}"))
                )?;
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
                    presentation,
                    "Continue within this line? [Y/n] ",
                    Some(true),
                )? {
                    localize_byte(
                        path,
                        line,
                        full_digest,
                        presentation,
                        input,
                        output,
                        diagnostics,
                    )?;
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
    full_digest: bool,
    presentation: Presentation,
    input: &mut impl BufRead,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<(), WorkflowError> {
    let palette = presentation.output;
    let local = inspect_line(path, line)?;
    let local_byte_len = local.map_or(0, |info| info.byte_len);
    writeln!(output)?;
    writeln!(
        output,
        "Line {} size: {} bytes (including any CR/LF)",
        palette.bold(line),
        palette.bold(local_byte_len)
    )?;
    let other_byte_len = read_count(
        input,
        output,
        diagnostics,
        presentation,
        "Enter the other copy's byte count for this line: ",
        "byte",
    )?;
    let mut search = ByteSearch::new(local_byte_len, other_byte_len)?;
    loop {
        match search.current_step() {
            ByteSearchStep::CompareThroughByte { byte } => {
                let fingerprint = fingerprint_line_prefix(path, line, byte)?;
                writeln!(output)?;
                writeln!(
                    output,
                    "{}",
                    palette.heading(format_args!("Compare line {line} through byte {byte}:"))
                )?;
                writeln!(
                    output,
                    "Fingerprint: {}",
                    palette.bold(format_fingerprint(fingerprint, full_digest))
                )?;
                let matched = read_match(
                    input,
                    output,
                    diagnostics,
                    presentation,
                    "Does this fingerprint match? [y/n] ",
                )?;
                search.record_result(matched)?;
            }
            ByteSearchStep::DifferenceAtByte { byte } => {
                let character = utf8_character_position(path, line, byte)?;
                writeln!(output)?;
                let mut result = format!("First divergence: line {line}, byte {byte}");
                if let Some(character) = character {
                    result.push_str(&format!(" (UTF-8 character {character})"));
                }
                writeln!(output, "{}", palette.difference(result))?;
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

/// Formats the first 8 lowercase hexadecimal digits, or all 64 in full-digest mode.
fn format_fingerprint(fingerprint: Fingerprint, full_digest: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = fingerprint.as_bytes();
    let displayed = if full_digest { &bytes[..] } else { &bytes[..4] };
    let mut formatted = String::with_capacity(displayed.len() * 2);
    for &byte in displayed {
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
    palette: Palette,
    prompt: &str,
) -> Result<String, WorkflowError> {
    write!(output, "{}", palette.prompt(prompt))?;
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
    presentation: Presentation,
    prompt: &str,
) -> Result<bool, WorkflowError> {
    read_yes_no(input, output, diagnostics, presentation, prompt, None)
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
    presentation: Presentation,
    prompt: &str,
    default: Option<bool>,
) -> Result<bool, WorkflowError> {
    loop {
        let answer = read_prompt(input, output, presentation.output, prompt)?;
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
            "{} invalid answer; enter y, yes, n, or no",
            presentation.diagnostics.error("paircomp:")
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
    presentation: Presentation,
    prompt: &str,
    kind: &str,
) -> Result<u64, WorkflowError> {
    loop {
        let answer = read_prompt(input, output, presentation.output, prompt)?;
        let answer = answer.trim();
        if !answer.is_empty() && answer.bytes().all(|byte| byte.is_ascii_digit()) {
            if let Ok(count) = answer.parse() {
                return Ok(count);
            }
        }
        writeln!(
            diagnostics,
            "{} invalid {kind} count; enter a nonnegative decimal u64",
            presentation.diagnostics.error("paircomp:")
        )?;
    }
}
