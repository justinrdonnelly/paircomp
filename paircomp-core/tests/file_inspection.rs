use paircomp_core::{
    fingerprint_file, fingerprint_through_line, inspect_file, Error, Fingerprint, LineSearch,
    SearchStep,
};
use std::fs;
use std::path::{Path, PathBuf};
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
            let dir = std::env::temp_dir().join(format!(
                "paircomp-core-test-{}-{number}",
                std::process::id()
            ));
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

fn assert_hash(actual: Fingerprint, bytes: &[u8]) {
    assert_eq!(actual.as_bytes(), blake3::hash(bytes).as_bytes());
}

#[test]
fn full_file_fingerprints_match_exact_bytes() {
    let first = Fixture::new(b"first\nsecond\n");
    let identical = Fixture::new(b"first\nsecond\n");
    let changed = Fixture::new(b"first\nseconD\n");

    let fingerprint = fingerprint_file(first.path()).unwrap();
    assert_eq!(fingerprint, fingerprint_file(identical.path()).unwrap());
    assert_ne!(fingerprint, fingerprint_file(changed.path()).unwrap());
    assert_eq!(fingerprint, inspect_file(first.path()).unwrap().fingerprint);
    assert_hash(fingerprint, b"first\nsecond\n");
}

#[test]
fn line_counts_follow_raw_lf_rules() {
    for (bytes, expected_lines) in [
        (&b""[..], 0),
        (&b"\n"[..], 1),
        (&b"a"[..], 1),
        (&b"a\n"[..], 1),
        (&b"a\n\n"[..], 2),
        (&b"a\r\n"[..], 1),
        (&b"a\rb"[..], 1),
    ] {
        let fixture = Fixture::new(bytes);
        let info = inspect_file(fixture.path()).unwrap();
        assert_eq!(info.byte_len, bytes.len() as u64, "bytes: {bytes:?}");
        assert_eq!(info.line_count, expected_lines, "bytes: {bytes:?}");
        assert_hash(info.fingerprint, bytes);
        assert_hash(fingerprint_through_line(fixture.path(), 0).unwrap(), b"");
        assert_eq!(
            fingerprint_through_line(fixture.path(), u64::MAX).unwrap(),
            info.fingerprint,
            "bytes: {bytes:?}"
        );
    }
}

#[test]
fn prefix_fingerprints_include_exactly_the_requested_lines() {
    let fixture = Fixture::new(b"ab\nc\r\nd");
    for (line, expected_bytes) in [
        (0, &b""[..]),
        (1, &b"ab\n"[..]),
        (2, &b"ab\nc\r\n"[..]),
        (3, &b"ab\nc\r\nd"[..]),
        (4, &b"ab\nc\r\nd"[..]),
    ] {
        assert_hash(
            fingerprint_through_line(fixture.path(), line).unwrap(),
            expected_bytes,
        );
    }
}

#[test]
fn prefix_boundary_can_cross_read_chunks() {
    let mut bytes = vec![b'x'; 8191];
    bytes.push(b'\n');
    bytes.extend(vec![b'y'; 10_000]);
    bytes.push(b'\n');
    bytes.extend(b"end");
    let fixture = Fixture::new(&bytes);

    assert_eq!(inspect_file(fixture.path()).unwrap().line_count, 3);
    assert_hash(
        fingerprint_through_line(fixture.path(), 1).unwrap(),
        &bytes[..8192],
    );
    assert_hash(
        fingerprint_through_line(fixture.path(), 2).unwrap(),
        &bytes[..18_193],
    );
    assert_hash(fingerprint_through_line(fixture.path(), 3).unwrap(), &bytes);
}

#[test]
fn final_newline_and_crlf_are_distinct_bytes() {
    let no_final_lf = Fixture::new(b"a");
    let final_lf = Fixture::new(b"a\n");
    let crlf = Fixture::new(b"a\r\n");

    assert_eq!(inspect_file(no_final_lf.path()).unwrap().line_count, 1);
    assert_eq!(inspect_file(final_lf.path()).unwrap().line_count, 1);
    assert_eq!(inspect_file(crlf.path()).unwrap().line_count, 1);
    assert_ne!(
        fingerprint_file(no_final_lf.path()).unwrap(),
        fingerprint_file(final_lf.path()).unwrap()
    );
    assert_ne!(
        fingerprint_file(final_lf.path()).unwrap(),
        fingerprint_file(crlf.path()).unwrap()
    );
    assert_hash(fingerprint_through_line(crlf.path(), 1).unwrap(), b"a\r\n");
}

#[test]
fn invalid_utf8_is_counted_and_hashed_without_decoding() {
    let bytes = b"\xff\r\n\x80\n\xfe";
    let fixture = Fixture::new(bytes);

    let info = inspect_file(fixture.path()).unwrap();
    assert_eq!(info.byte_len, 6);
    assert_eq!(info.line_count, 3);
    assert_hash(info.fingerprint, bytes);
    assert_hash(
        fingerprint_through_line(fixture.path(), 2).unwrap(),
        b"\xff\r\n\x80\n",
    );
}

#[test]
fn missing_and_non_regular_inputs_return_errors() {
    let fixture = Fixture::new(b"present");
    let missing = fixture.dir.join("missing");
    assert!(matches!(inspect_file(&missing), Err(Error::Io(_))));
    assert!(matches!(fingerprint_file(&missing), Err(Error::Io(_))));
    assert!(matches!(
        fingerprint_through_line(&missing, 0),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        inspect_file(&fixture.dir),
        Err(Error::NotRegularFile)
    ));
}

#[test]
fn search_state_follows_midpoint_bounds_and_rejects_invalid_operations() {
    assert!(matches!(
        LineSearch::new(0, 0),
        Err(Error::EmptyFilesCannotDiffer)
    ));

    let mut search = LineSearch::new(4, 4).unwrap();
    assert_eq!(
        search.current_step(),
        SearchStep::CompareThroughLine { line: 2 }
    );
    assert_eq!(
        search.current_step(),
        SearchStep::CompareThroughLine { line: 2 }
    );
    search.record_result(true).unwrap();
    assert_eq!(
        search.current_step(),
        SearchStep::CompareThroughLine { line: 3 }
    );
    search.record_result(true).unwrap();
    assert_eq!(
        search.current_step(),
        SearchStep::DifferenceAtLine { line: 4 }
    );
    assert!(matches!(
        search.record_result(false),
        Err(Error::SearchAlreadyComplete)
    ));
    assert_eq!(
        search.current_step(),
        SearchStep::DifferenceAtLine { line: 4 }
    );

    let mut first_line = LineSearch::new(1, 0).unwrap();
    assert_eq!(
        first_line.current_step(),
        SearchStep::DifferenceAtLine { line: 1 }
    );
    assert!(matches!(
        first_line.record_result(true),
        Err(Error::SearchAlreadyComplete)
    ));
}

#[test]
fn search_midpoint_does_not_overflow_at_maximum_line_count() {
    let mut search = LineSearch::new(0, u64::MAX).unwrap();
    assert_eq!(
        search.current_step(),
        SearchStep::CompareThroughLine { line: 1 << 63 }
    );
    for _ in 0..63 {
        search.record_result(true).unwrap();
    }
    assert_eq!(
        search.current_step(),
        SearchStep::DifferenceAtLine { line: u64::MAX }
    );
}

fn paired_search(first_bytes: &[u8], second_bytes: &[u8]) -> Option<u64> {
    let first = Fixture::new(first_bytes);
    let second = Fixture::new(second_bytes);
    let first_info = inspect_file(first.path()).unwrap();
    let second_info = inspect_file(second.path()).unwrap();
    if first_info.fingerprint == second_info.fingerprint {
        return None;
    }

    let mut first_search = LineSearch::new(first_info.line_count, second_info.line_count).unwrap();
    let mut second_search = LineSearch::new(second_info.line_count, first_info.line_count).unwrap();
    for _ in 0..64 {
        let first_step = first_search.current_step();
        assert_eq!(first_step, first_search.current_step());
        assert_eq!(first_step, second_search.current_step());
        match first_step {
            SearchStep::CompareThroughLine { line } => {
                let matched = fingerprint_through_line(first.path(), line).unwrap()
                    == fingerprint_through_line(second.path(), line).unwrap();
                first_search.record_result(matched).unwrap();
                second_search.record_result(matched).unwrap();
            }
            SearchStep::DifferenceAtLine { line } => return Some(line),
        }
    }
    panic!("paired searches did not terminate");
}

#[test]
fn paired_instances_request_the_same_prefix_and_find_the_first_difference() {
    for (first, second, expected_line) in [
        (&b"a\nb\nc\n"[..], &b"X\nb\nc\n"[..], 1),
        (&b"a\nb\nc\n"[..], &b"a\nX\nc\n"[..], 2),
        (&b"a\nb\nc\n"[..], &b"a\nb\nX\n"[..], 3),
        (&b"a\nb\nc\n"[..], &b"X\na\nb\nc\n"[..], 1),
        (&b"a\nb\nc\n"[..], &b"a\nX\nb\nc\n"[..], 2),
        (&b"a\nb\nc\n"[..], &b"a\nb\nX\nc\n"[..], 3),
        (&b"a\nb\n"[..], &b"a\nb\nc\n"[..], 3),
        (&b""[..], &b"a"[..], 1),
        (&b"a"[..], &b"a\n"[..], 1),
        (&b"a\r\nb\n"[..], &b"a\nb\n"[..], 1),
        (&b"\xff\nsame\n"[..], &b"\xff\n\x80\n"[..], 2),
        (&b"old"[..], &b"new"[..], 1),
    ] {
        assert_eq!(
            paired_search(first, second),
            Some(expected_line),
            "first: {first:?}, second: {second:?}"
        );
    }
}

#[test]
fn identical_files_complete_at_the_whole_file_comparison() {
    assert_eq!(paired_search(b"same\nbytes\n", b"same\nbytes\n"), None);
    assert_eq!(paired_search(b"", b""), None);
}
