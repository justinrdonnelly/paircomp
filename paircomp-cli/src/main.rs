use clap::Parser;
use paircomp_core::{fingerprint_through_line, inspect_file, Fingerprint, LineSearch, SearchStep};
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

    let other_line_count = read_line_count(input, output)?;
    let mut search = LineSearch::new(info.line_count, other_line_count)?;
    loop {
        match search.current_step() {
            SearchStep::CompareThroughLine { line } => {
                let fingerprint = fingerprint_through_line(path, line)?;
                writeln!(output)?;
                writeln!(output, "Compare through line {line}:")?;
                writeln!(output, "Fingerprint: {}", format_fingerprint(fingerprint))?;
                let matched = read_match(input, output, "Does this fingerprint match? [y/N] ")?;
                search.record_result(matched)?;
            }
            SearchStep::DifferenceAtLine { line } => {
                writeln!(output)?;
                writeln!(output, "First divergence: line {line}")?;
                if line > info.line_count {
                    writeln!(output, "The local file ends before line {line}.")?;
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

fn format_fingerprint(fingerprint: Fingerprint) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut formatted = String::with_capacity(64);
    for &byte in fingerprint.as_bytes() {
        formatted.push(char::from(HEX[usize::from(byte >> 4)]));
        formatted.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    formatted
}

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
    let answer = read_prompt(input, output, prompt)?;
    let answer = answer.trim();
    if answer.is_empty() || answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
        Ok(false)
    } else if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
        Ok(true)
    } else {
        Err(WorkflowError::InvalidAnswer)
    }
}

fn read_line_count(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<u64, WorkflowError> {
    let answer = read_prompt(input, output, "Line count displayed by the other copy: ")?;
    let answer = answer.trim();
    if answer.is_empty() || !answer.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WorkflowError::InvalidLineCount);
    }
    answer.parse().map_err(|_| WorkflowError::InvalidLineCount)
}
