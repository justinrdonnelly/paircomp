use paircomp_core::{fingerprint_line_prefix, fingerprint_through_line, inspect_file, Fingerprint};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
    file: PathBuf,
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = loop {
            let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("paircomp-cli-test-{}-{number}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test directory: {error}"),
            }
        };
        let file = dir.join("input");
        fs::write(&file, bytes).unwrap();
        Self { dir, file }
    }

    fn path(&self) -> &Path {
        &self.file
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn invoke(path: &Path, answers: &str) -> Output {
    invoke_with_args(path, &[], answers)
}

fn invoke_with_args(path: &Path, args: &[&str], answers: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_paircomp"))
        .arg(path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answers.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn digest_hex(fingerprint: Fingerprint) -> String {
    fingerprint
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fingerprints(output: &Output) -> Vec<String> {
    stdout(output)
        .lines()
        .filter_map(|line| line.strip_prefix("Fingerprint: "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn help_version_and_argument_errors() {
    let binary = env!("CARGO_BIN_EXE_paircomp");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("Usage: paircomp [OPTIONS] <FILE>"));
    assert!(stdout(&help).contains("--full-digest"));
    assert!(stdout(&help).contains("Display all 64 hexadecimal digits instead of the first 8"));

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert_eq!(version.status.code(), Some(0));
    assert!(stdout(&version).contains(concat!("paircomp ", env!("CARGO_PKG_VERSION"))));

    for args in [&[][..], &["--full-digest"][..]] {
        let missing = Command::new(binary).args(args).output().unwrap();
        assert_eq!(missing.status.code(), Some(2));
        assert!(stderr(&missing).contains("<FILE>"));
    }

    let extra = Command::new(binary)
        .args(["first", "second"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}

#[test]
fn confirmed_whole_file_match_exits_successfully() {
    let fixture = Fixture::new(b"same\nbytes\n");
    let expected_digest = digest_hex(inspect_file(fixture.path()).unwrap().fingerprint);
    for (args, length) in [(&[][..], 8), (&["--full-digest"][..], 64)] {
        let output = invoke_with_args(fixture.path(), args, " YES \n");
        assert_eq!(output.status.code(), Some(0));
        let text = stdout(&output);
        assert!(text.contains("Lines: 2\nSize: 11 bytes\n"));
        assert_eq!(fingerprints(&output), [&expected_digest[..length]]);
        assert!(text.contains("Use the same fingerprint display mode on both copies."));
        assert!(text.contains("Keep both files unchanged"));
        assert!(text.contains("Files match."));
        assert!(!text.contains("Line count displayed by the other copy"));
        assert!(!text.contains("Continue within this line"));
        assert!(!text.contains("Compare line"));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn paired_cli_uses_selected_fingerprint_length_at_every_stage() {
    let fixture = Fixture::new("same\ncafé".as_bytes());
    let other = Fixture::new("same\ncafè".as_bytes());
    let expected = [
        inspect_file(fixture.path()).unwrap().fingerprint,
        fingerprint_through_line(fixture.path(), 1).unwrap(),
        fingerprint_line_prefix(fixture.path(), 2, 3).unwrap(),
        fingerprint_line_prefix(fixture.path(), 2, 4).unwrap(),
    ]
    .map(digest_hex);
    for (args, length) in [(&[][..], 8), (&["--full-digest"][..], 64)] {
        let output = invoke_with_args(fixture.path(), args, "n\n2\ny\n\n5\ny\ny\n");
        let other_output = invoke_with_args(other.path(), args, "n\n2\ny\n\n5\ny\ny\n");
        let local_hashes = fingerprints(&output);
        let other_hashes = fingerprints(&other_output);
        assert_eq!(
            local_hashes,
            expected
                .iter()
                .map(|digest| &digest[..length])
                .collect::<Vec<_>>()
        );
        assert_eq!(other_hashes.len(), 4);
        assert_ne!(local_hashes[0], other_hashes[0]);
        assert_eq!(local_hashes[1..], other_hashes[1..]);
        for output in [&output, &other_output] {
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stderr.is_empty());
            assert!(fingerprints(output)
                .iter()
                .all(|digest| digest.len() == length));
            let text = stdout(output);
            assert!(text.contains("Compare through line 1:"));
            assert!(text.contains("Compare line 2 through byte 3:"));
            assert!(text.contains("Compare line 2 through byte 4:"));
            assert!(text.contains("First divergence: line 2, byte 5 (UTF-8 character 4)"));
        }
    }
}

#[test]
fn equal_counts_follow_core_search_and_report_a_difference() {
    let fixture = Fixture::new(b"a\nb\nlocal\nd\n");
    let output = invoke(fixture.path(), "n\n 4 \ny\n NO \nn\n");
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("Line count displayed by the other copy: "));
    assert!(text.contains("Compare through line 2:"));
    assert!(text.contains("Compare through line 3:"));
    let prefix_digest = digest_hex(fingerprint_through_line(fixture.path(), 2).unwrap());
    let prefix_digest = &prefix_digest[..8];
    assert!(text.contains(&format!(
        "Compare through line 2:\nFingerprint: {prefix_digest}\n"
    )));
    assert!(text.contains("First divergence: line 3"));
    assert!(!text.contains("local file ends before"));
    assert!(text.contains("Continue within this line? [Y/n]"));
    assert!(!text.contains("Byte count displayed"));
    assert!(output.stderr.is_empty());
}

#[test]
fn beyond_eof_is_reported() {
    let fixture = Fixture::new(b"a\n");
    let output = invoke(fixture.path(), "n\n2\ny\nn\n");
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("Compare through line 1:"));
    assert!(text.contains("First divergence: line 2"));
    assert!(text.contains("The local file ends before line 2."));
    assert!(output.stderr.is_empty());
}

#[test]
fn empty_file_can_localize_a_nonempty_other_file() {
    let fixture = Fixture::new(b"");
    let output = invoke(fixture.path(), "no\n1\nn\n");
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("Lines: 0"));
    assert!(text.contains("First divergence: line 1"));
    assert!(text.contains("The local file ends before line 1."));
    assert!(!text.contains("Compare through line"));
}

#[test]
fn zero_other_line_count_is_valid_for_a_nonempty_local_file() {
    let fixture = Fixture::new(b"a\n");
    let output = invoke(fixture.path(), "no\n0\nn\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("First divergence: line 1"));
    assert!(output.stderr.is_empty());
}

#[test]
fn eof_aborts_even_after_invalid_input() {
    let fixture = Fixture::new(b"a\n");
    for (answers, diagnostic) in [
        ("", "input ended"),
        ("perhaps\n", "invalid answer"),
        ("no\n", "input ended"),
        ("no\n\n", "invalid line count"),
        ("no\n-1\n", "invalid line count"),
        ("no\n18446744073709551616\n", "invalid line count"),
        ("no\n2\n", "input ended"),
        ("no\n2\nmaybe\n", "invalid answer"),
    ] {
        let output = invoke(fixture.path(), answers);
        assert_eq!(output.status.code(), Some(2), "answers: {answers:?}");
        assert!(stderr(&output).contains(diagnostic), "answers: {answers:?}");
        assert!(stderr(&output).ends_with("input ended before the comparison was complete\n"));
    }
}

#[test]
fn fingerprint_prompts_reject_blank_answers_at_every_stage() {
    let fixture = Fixture::new(b"ab\ncd\n");
    for (prefix, prompt, forbidden_result) in [
        (
            "",
            "Does this fingerprint match the other copy? [y/n] ",
            "Line count displayed by the other copy:",
        ),
        (
            "n\n2\n",
            "Does this fingerprint match? [y/n] ",
            "First divergence:",
        ),
        (
            "n\n2\nn\ny\n3\n",
            "Does this fingerprint match? [y/n] ",
            "First divergence: line 1, byte",
        ),
    ] {
        for blank in ["\n", " \t\n"] {
            let answers = format!("{prefix}{blank}");
            let output = invoke(fixture.path(), &answers);
            assert_eq!(output.status.code(), Some(2), "answers: {answers:?}");
            assert!(stderr(&output).contains("invalid answer; enter y, yes, n, or no"));
            assert!(stderr(&output).ends_with("input ended before the comparison was complete\n"));
            let text = stdout(&output);
            assert!(text.ends_with(&prompt.repeat(2)), "answers: {answers:?}");
            assert!(!text.contains(forbidden_result), "answers: {answers:?}");
        }
    }
}

#[test]
fn invalid_input_repeats_each_prompt_and_accepts_a_corrected_answer() {
    let fixture = Fixture::new(b"ab\ncd\n");
    let invalid_answers: &[&str] = &["", " \t", "maybe"];
    let invalid_counts: &[&str] = &["", " \t", "no", "-1", "+2", "1.5", "18446744073709551616"];
    for (prefix, suffix, prompt, invalid, diagnostic, status) in [
        (
            "",
            " YES \n",
            "Does this fingerprint match the other copy? [y/n] ",
            invalid_answers,
            "invalid answer; enter y, yes, n, or no",
            0,
        ),
        (
            "n\n",
            " 2 \ny\n\n3\nn\ny\n",
            "Line count displayed by the other copy: ",
            invalid_counts,
            "invalid line count; enter a nonnegative decimal u64",
            1,
        ),
        (
            "n\n2\n",
            " YES \n\n3\nn\ny\n",
            "Does this fingerprint match? [y/n] ",
            invalid_answers,
            "invalid answer; enter y, yes, n, or no",
            1,
        ),
        (
            "n\n2\ny\n",
            " \t\n3\nn\ny\n",
            "Continue within this line? [Y/n] ",
            &["maybe", "1"],
            "invalid answer; enter y, yes, n, or no",
            1,
        ),
        (
            "n\n2\ny\n",
            " NO \n",
            "Continue within this line? [Y/n] ",
            &["maybe", "1"],
            "invalid answer; enter y, yes, n, or no",
            1,
        ),
        (
            "n\n2\ny\n\n",
            " 3 \nn\ny\n",
            "Byte count displayed for this line by the other copy: ",
            invalid_counts,
            "invalid byte count; enter a nonnegative decimal u64",
            1,
        ),
        (
            "n\n2\ny\n\n3\n",
            " NO \ny\n",
            "Does this fingerprint match? [y/n] ",
            invalid_answers,
            "invalid answer; enter y, yes, n, or no",
            1,
        ),
    ] {
        let clean = invoke(fixture.path(), &format!("{prefix}{suffix}"));
        assert_eq!(clean.status.code(), Some(status));
        assert!(clean.stderr.is_empty());
        let answers = format!("{prefix}{}\n{suffix}", invalid.join("\n"));
        let recovered = invoke(fixture.path(), &answers);
        assert_eq!(
            recovered.status.code(),
            Some(status),
            "answers: {answers:?}"
        );
        assert_eq!(
            stderr(&recovered),
            format!("paircomp: {diagnostic}\n").repeat(invalid.len()),
            "answers: {answers:?}"
        );
        let clean_text = stdout(&clean);
        let recovered_text = stdout(&recovered);
        assert_eq!(
            recovered_text.matches(prompt).count(),
            clean_text.matches(prompt).count() + invalid.len(),
            "answers: {answers:?}"
        );
        // Only the repeated prompt may differ: fingerprints, requested positions,
        // and final results must be unchanged by invalid answers.
        assert_eq!(
            recovered_text.replace(prompt, ""),
            clean_text.replace(prompt, ""),
            "answers: {answers:?}"
        );
    }
}

#[test]
fn unterminated_answers_abort_at_every_prompt() {
    let fixture = Fixture::new(b"a\n");
    for answers in [
        "yes",
        "  ",
        "no\n2",
        "no\n2\ny",
        "maybe\nyes",
        "no\n-1\n2",
        "no\n2\nmaybe\ny",
    ] {
        let output = invoke(fixture.path(), answers);
        assert_eq!(output.status.code(), Some(2), "answers: {answers:?}");
        assert!(
            stderr(&output).contains("input ended"),
            "answers: {answers:?}"
        );
        assert!(!stdout(&output).contains("Files match."));
        assert!(!stdout(&output).contains("First divergence:"));
    }
}

#[test]
fn inconsistent_empty_mismatch_and_invalid_files_exit_with_status_two() {
    let fixture = Fixture::new(b"");
    let mismatch = invoke(fixture.path(), "no\n0\n");
    assert_eq!(mismatch.status.code(), Some(2));
    assert!(stderr(&mismatch).contains("two empty files cannot differ"));

    let directory = invoke(&fixture.dir, "");
    assert_eq!(directory.status.code(), Some(2));
    assert!(stderr(&directory).contains("not a regular file"));

    let missing = invoke(&fixture.dir.join("missing"), "");
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("file I/O failed"));
}

#[cfg(unix)]
#[test]
fn non_utf8_file_path_is_accepted() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::new(b"bytes\n");
    let path = fixture.dir.join(OsString::from_vec(b"file-\xff".to_vec()));
    fs::write(&path, b"bytes\n").unwrap();
    let output = invoke(&path, "yes\n");
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Files match."));
}

#[test]
fn paired_cli_instances_stay_aligned_after_retries_and_default_continuation() {
    let first = Fixture::new("same\ncafé".as_bytes());
    let second = Fixture::new("same\ncafè".as_bytes());
    let first_output = invoke(
        first.path(),
        "\nn\nbad\n2\n\nYES\nmaybe\n \n-1\n 5 \nmaybe\ny\ny\n",
    );
    let second_output = invoke(second.path(), "n\n2\ny\n \n 5 \ny\ny\n");
    assert_eq!(
        stderr(&first_output),
        concat!(
            "paircomp: invalid answer; enter y, yes, n, or no\n",
            "paircomp: invalid line count; enter a nonnegative decimal u64\n",
            "paircomp: invalid answer; enter y, yes, n, or no\n",
            "paircomp: invalid answer; enter y, yes, n, or no\n",
            "paircomp: invalid byte count; enter a nonnegative decimal u64\n",
            "paircomp: invalid answer; enter y, yes, n, or no\n",
        )
    );
    assert!(second_output.stderr.is_empty());
    for output in [&first_output, &second_output] {
        assert_eq!(output.status.code(), Some(1));
        let text = stdout(output);
        assert!(text.contains("Line 2 size: 5 bytes (including any CR/LF)"));
        assert!(text.contains("Byte count displayed for this line by the other copy:"));
        assert!(text.contains("Compare line 2 through byte 3:"));
        assert!(text.contains("Compare line 2 through byte 4:"));
        assert!(text.contains("First divergence: line 2, byte 5 (UTF-8 character 4)"));
    }
    // The whole files differ, but all three subsequent prefix comparisons match.
    let first_hashes = fingerprints(&first_output);
    let second_hashes = fingerprints(&second_output);
    assert_eq!(first_hashes.len(), 4);
    assert_ne!(first_hashes[0], second_hashes[0]);
    assert_eq!(first_hashes[1..], second_hashes[1..]);
}

#[test]
fn paired_cli_handles_a_missing_final_newline_and_an_absent_line() {
    let short = Fixture::new(b"a");
    let long = Fixture::new(b"a\n");
    let short_output = invoke(short.path(), "n\n1\n YES \n2\ny\n");
    let long_output = invoke(long.path(), "n\n1\ny\n1\ny\n");
    for output in [&short_output, &long_output] {
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        assert!(stdout(output).contains("First divergence: line 1, byte 2 (UTF-8 character 2)"));
    }
    assert!(
        stdout(&short_output).contains("The local line ends before byte 2; this byte is absent.")
    );
    assert!(!stdout(&long_output).contains("this byte is absent"));

    let short = Fixture::new(b"a\n");
    let long = Fixture::new(b"a\nb\n");
    let short_output = invoke(short.path(), "n\n2\ny\ny\n2\nn\n");
    let long_output = invoke(long.path(), "n\n1\ny\ny\n0\nn\n");
    for output in [&short_output, &long_output] {
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        assert!(stdout(output).contains("Compare line 2 through byte 1:"));
        assert!(stdout(output).contains("First divergence: line 2, byte 1"));
    }
    assert!(stdout(&short_output).contains("Line 2 size: 0 bytes"));
    assert!(
        stdout(&short_output).contains("The local file ends before line 2; this byte is absent.")
    );
    assert!(!stdout(&short_output).contains("UTF-8 character"));
    assert!(stdout(&long_output).contains("(UTF-8 character 1)"));
}

#[test]
fn invalid_utf8_reports_only_the_byte_position() {
    let fixture = Fixture::new(b"a\xff");
    let output = invoke(fixture.path(), "n\n1\ny\n2\ny\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(stdout(&output).contains("First divergence: line 1, byte 2\n"));
    assert!(!stdout(&output).contains("UTF-8 character"));
}

#[test]
fn within_line_eof_aborts_even_after_invalid_input() {
    let fixture = Fixture::new(b"ab");
    for (answers, diagnostic) in [
        ("n\n1\n", "input ended"),
        ("n\n1\ny", "input ended"),
        ("n\n1\nmaybe\n", "invalid answer"),
        ("n\n1\nmaybe\nn", "invalid answer"),
        ("n\n1\n\n", "input ended"),
        ("n\n1\ny\n2", "input ended"),
        ("n\n1\ny\n\n", "invalid byte count"),
        ("n\n1\ny\n-1\n", "invalid byte count"),
        ("n\n1\ny\n-1\n2", "invalid byte count"),
        ("n\n1\ny\n+2\n", "invalid byte count"),
        ("n\n1\ny\n1.5\n", "invalid byte count"),
        ("n\n1\ny\n18446744073709551616\n", "invalid byte count"),
        ("n\n1\ny\n2\n", "input ended"),
        ("n\n1\ny\n2\ny", "input ended"),
        ("n\n1\ny\n2\nmaybe\n", "invalid answer"),
        ("n\n1\ny\n2\nmaybe\ny", "invalid answer"),
    ] {
        let output = invoke(fixture.path(), answers);
        assert_eq!(output.status.code(), Some(2), "answers: {answers:?}");
        assert!(stderr(&output).contains(diagnostic), "answers: {answers:?}");
        assert!(stderr(&output).ends_with("input ended before the comparison was complete\n"));
        assert!(!stdout(&output).contains("First divergence: line 1, byte"));
    }
    let empty = Fixture::new(b"");
    let output = invoke(empty.path(), "n\n1\ny\n0\n");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("two zero-length lines cannot differ"));
}
