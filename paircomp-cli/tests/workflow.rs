use paircomp_core::{fingerprint_through_line, inspect_file};
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_paircomp"))
        .arg(path)
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

#[test]
fn help_version_and_argument_errors() {
    let binary = env!("CARGO_BIN_EXE_paircomp");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("Usage: paircomp <FILE>"));

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert_eq!(version.status.code(), Some(0));
    assert!(stdout(&version).contains("paircomp 0.1.0"));

    let missing = Command::new(binary).output().unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("<FILE>"));

    let extra = Command::new(binary)
        .args(["first", "second"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}

#[test]
fn confirmed_whole_file_match_exits_successfully() {
    let fixture = Fixture::new(b"same\nbytes\n");
    let output = invoke(fixture.path(), " YES \n");
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert!(text.contains("Lines: 2\nSize: 11 bytes\n"));
    let expected_digest = inspect_file(fixture.path())
        .unwrap()
        .fingerprint
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(text.contains(&format!("Fingerprint: {expected_digest}\n")));
    assert!(text.contains("Keep both files unchanged"));
    assert!(text.contains("Files match."));
    assert!(!text.contains("Line count displayed by the other copy"));
    assert!(output.stderr.is_empty());
}

#[test]
fn equal_counts_follow_core_search_and_report_a_difference() {
    let fixture = Fixture::new(b"a\nb\nlocal\nd\n");
    let output = invoke(fixture.path(), "n\n 4 \ny\n \n");
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("Line count displayed by the other copy: "));
    assert!(text.contains("Compare through line 2:"));
    assert!(text.contains("Compare through line 3:"));
    let prefix_digest = fingerprint_through_line(fixture.path(), 2)
        .unwrap()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(text.contains(&format!(
        "Compare through line 2:\nFingerprint: {prefix_digest}\n"
    )));
    assert!(text.contains("First divergence: line 3"));
    assert!(!text.contains("local file ends before"));
    assert!(output.stderr.is_empty());
}

#[test]
fn blank_answer_uses_no_default_and_beyond_eof_is_reported() {
    let fixture = Fixture::new(b"a\n");
    let output = invoke(fixture.path(), "  \n2\ny\n");
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
    let output = invoke(fixture.path(), "no\n1\n");
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
    let output = invoke(fixture.path(), "no\n0\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("First divergence: line 1"));
    assert!(output.stderr.is_empty());
}

#[test]
fn invalid_or_aborted_input_exits_with_status_two() {
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
