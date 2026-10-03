use paircomp_core::{
    fingerprint_file, fingerprint_line_prefix, fingerprint_through_line, inspect_file,
    inspect_line, utf8_character_position, ByteSearch, ByteSearchStep, Error, Fingerprint,
    LineSearch, LineSearchStep,
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
        LineSearchStep::CompareThroughLine { line: 2 }
    );
    assert_eq!(
        search.current_step(),
        LineSearchStep::CompareThroughLine { line: 2 }
    );
    search.record_result(true).unwrap();
    assert_eq!(
        search.current_step(),
        LineSearchStep::CompareThroughLine { line: 3 }
    );
    search.record_result(true).unwrap();
    assert_eq!(
        search.current_step(),
        LineSearchStep::DifferenceAtLine { line: 4 }
    );
    assert!(matches!(
        search.record_result(false),
        Err(Error::SearchAlreadyComplete)
    ));
    assert_eq!(
        search.current_step(),
        LineSearchStep::DifferenceAtLine { line: 4 }
    );

    let mut first_line = LineSearch::new(1, 0).unwrap();
    assert_eq!(
        first_line.current_step(),
        LineSearchStep::DifferenceAtLine { line: 1 }
    );
    assert!(matches!(
        first_line.record_result(true),
        Err(Error::SearchAlreadyComplete)
    ));
}

#[test]
fn search_midpoint_does_not_overflow_at_maximum_line_count() {
    for counts in [
        (u64::MAX, u64::MAX),
        (u64::MAX - 1, u64::MAX),
        (u64::MAX, u64::MAX - 1),
    ] {
        let mut search = LineSearch::new(counts.0, counts.1).unwrap();
        assert_eq!(
            search.current_step(),
            LineSearchStep::CompareThroughLine { line: 1 << 63 }
        );
        for _ in 0..63 {
            search.record_result(true).unwrap();
        }
        assert_eq!(
            search.current_step(),
            LineSearchStep::DifferenceAtLine { line: u64::MAX }
        );
    }
    for counts in [(0, u64::MAX), (u64::MAX, 0)] {
        let search = LineSearch::new(counts.0, counts.1).unwrap();
        assert_eq!(
            search.current_step(),
            LineSearchStep::DifferenceAtLine { line: 1 }
        );
    }
}

fn paired_search(first_bytes: &[u8], second_bytes: &[u8]) -> Option<(u64, Vec<u64>)> {
    let first = Fixture::new(first_bytes);
    let second = Fixture::new(second_bytes);
    let first_info = inspect_file(first.path()).unwrap();
    let second_info = inspect_file(second.path()).unwrap();
    if first_info.fingerprint == second_info.fingerprint {
        return None;
    }

    let mut first_search = LineSearch::new(first_info.line_count, second_info.line_count).unwrap();
    let mut second_search = LineSearch::new(second_info.line_count, first_info.line_count).unwrap();
    let mut comparisons = Vec::new();
    for _ in 0..64 {
        let first_step = first_search.current_step();
        assert_eq!(first_step, first_search.current_step());
        assert_eq!(first_step, second_search.current_step());
        match first_step {
            LineSearchStep::CompareThroughLine { line } => {
                assert!(line <= first_info.line_count.min(second_info.line_count));
                comparisons.push(line);
                let matched = fingerprint_through_line(first.path(), line).unwrap()
                    == fingerprint_through_line(second.path(), line).unwrap();
                first_search.record_result(matched).unwrap();
                second_search.record_result(matched).unwrap();
            }
            LineSearchStep::DifferenceAtLine { line } => return Some((line, comparisons)),
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
            paired_search(first, second).map(|(line, _)| line),
            Some(expected_line),
            "first: {first:?}, second: {second:?}"
        );
    }
}

#[test]
fn unequal_line_counts_only_compare_lines_present_in_both_files() {
    let long = b"a\n".repeat(220);
    for (short, expected_line, comparisons) in [
        (&b""[..], 1, vec![]),
        (&b"a\n"[..], 2, vec![1]),
        (&b"a\na\n"[..], 3, vec![2]),
        (&b"X\na\n"[..], 1, vec![2, 1]),
        (&b"a\nX\n"[..], 2, vec![2, 1]),
        (&b"a\na"[..], 2, vec![2, 1]),
    ] {
        for (first, second) in [(short, long.as_slice()), (long.as_slice(), short)] {
            assert_eq!(
                paired_search(first, second),
                Some((expected_line, comparisons.clone()))
            );
        }
    }
}

#[test]
fn identical_files_complete_at_the_whole_file_comparison() {
    assert_eq!(paired_search(b"same\nbytes\n", b"same\nbytes\n"), None);
    assert_eq!(paired_search(b"", b""), None);
}

#[test]
fn line_inspection_and_byte_prefixes_respect_line_boundaries() {
    let fixture = Fixture::new(b"prior\nab\r\n\xff\nlast");
    for (line, bytes) in [
        (1, &b"prior\n"[..]),
        (2, &b"ab\r\n"[..]),
        (3, &b"\xff\n"[..]),
        (4, &b"last"[..]),
        (5, &b""[..]),
        (u64::MAX, &b""[..]),
    ] {
        let info = inspect_line(fixture.path(), line).unwrap();
        assert_eq!(
            info.map(|info| info.byte_len),
            (!bytes.is_empty()).then_some(bytes.len() as u64)
        );
        for count in [0, 1, 2, 3, 4, 5, 6, u64::MAX] {
            let end = count.min(bytes.len() as u64) as usize;
            assert_hash(
                fingerprint_line_prefix(fixture.path(), line, count).unwrap(),
                &bytes[..end],
            );
        }
    }
    for bytes in [&b""[..], &b"\n"[..], &b"a\n"[..], &b"a\r"[..]] {
        let fixture = Fixture::new(bytes);
        assert_eq!(
            inspect_line(fixture.path(), 1)
                .unwrap()
                .map(|info| info.byte_len),
            (!bytes.is_empty()).then_some(bytes.len() as u64)
        );
        assert_eq!(inspect_line(fixture.path(), 2).unwrap(), None);
    }
    assert!(matches!(
        inspect_line(fixture.path(), 0),
        Err(Error::InvalidLineNumber)
    ));
    assert!(matches!(
        fingerprint_line_prefix(fixture.path(), 0, 0),
        Err(Error::InvalidLineNumber)
    ));
    assert!(matches!(
        utf8_character_position(fixture.path(), 0, 1),
        Err(Error::InvalidLineNumber)
    ));
    assert!(matches!(
        utf8_character_position(fixture.path(), 1, 0),
        Err(Error::InvalidBytePosition)
    ));
    assert!(matches!(
        utf8_character_position(fixture.path(), 1, 8),
        Err(Error::InvalidBytePosition)
    ));
    assert!(matches!(
        inspect_line(&fixture.dir, 1),
        Err(Error::NotRegularFile)
    ));
    assert!(matches!(
        fingerprint_line_prefix(&fixture.dir.join("missing"), 1, 0),
        Err(Error::Io(_))
    ));
}

#[test]
fn byte_search_state_is_stable_checked_and_overflow_safe() {
    assert!(matches!(
        ByteSearch::new(0, 0),
        Err(Error::EmptyLinesCannotDiffer)
    ));
    let mut search = ByteSearch::new(7, 7).unwrap();
    assert_eq!(
        search.current_step(),
        ByteSearchStep::CompareThroughByte { byte: 4 }
    );
    assert_eq!(search.current_step(), search.current_step());
    search.record_result(true).unwrap();
    assert_eq!(
        search.current_step(),
        ByteSearchStep::CompareThroughByte { byte: 6 }
    );
    search.record_result(false).unwrap();
    assert_eq!(
        search.current_step(),
        ByteSearchStep::CompareThroughByte { byte: 5 }
    );
    search.record_result(false).unwrap();
    assert_eq!(
        search.current_step(),
        ByteSearchStep::DifferenceAtByte { byte: 5 }
    );
    assert!(matches!(
        search.record_result(true),
        Err(Error::SearchAlreadyComplete)
    ));
    assert_eq!(
        search.current_step(),
        ByteSearchStep::DifferenceAtByte { byte: 5 }
    );

    let mut single = ByteSearch::new(0, 1).unwrap();
    assert_eq!(
        single.current_step(),
        ByteSearchStep::DifferenceAtByte { byte: 1 }
    );
    assert!(matches!(
        single.record_result(false),
        Err(Error::SearchAlreadyComplete)
    ));
    for counts in [(0, u64::MAX), (u64::MAX, 0)] {
        let search = ByteSearch::new(counts.0, counts.1).unwrap();
        assert_eq!(
            search.current_step(),
            ByteSearchStep::DifferenceAtByte { byte: 1 }
        );
    }
    for counts in [
        (u64::MAX, u64::MAX),
        (u64::MAX - 1, u64::MAX),
        (u64::MAX, u64::MAX - 1),
    ] {
        for matched in [false, true] {
            let mut search = ByteSearch::new(counts.0, counts.1).unwrap();
            assert_eq!(
                search.current_step(),
                ByteSearchStep::CompareThroughByte { byte: 1 << 63 }
            );
            while matches!(
                search.current_step(),
                ByteSearchStep::CompareThroughByte { .. }
            ) {
                search.record_result(matched).unwrap();
            }
            assert_eq!(
                search.current_step(),
                ByteSearchStep::DifferenceAtByte {
                    byte: if matched { u64::MAX } else { 1 }
                }
            );
        }
    }
}

fn paired_byte_search(
    first: &[u8],
    second: &[u8],
    expected_line: u64,
    expected_byte: u64,
) -> Vec<u64> {
    let (line, _) = paired_search(first, second).unwrap();
    assert_eq!(line, expected_line);
    let first = Fixture::new(first);
    let second = Fixture::new(second);
    let first_len = inspect_line(first.path(), line)
        .unwrap()
        .map_or(0, |info| info.byte_len);
    let second_len = inspect_line(second.path(), line)
        .unwrap()
        .map_or(0, |info| info.byte_len);
    let mut a = ByteSearch::new(first_len, second_len).unwrap();
    let mut b = ByteSearch::new(second_len, first_len).unwrap();
    let mut comparisons = Vec::new();
    for _ in 0..64 {
        let step = a.current_step();
        assert_eq!(step, a.current_step());
        assert_eq!(step, b.current_step());
        match step {
            ByteSearchStep::CompareThroughByte { byte } => {
                assert!(byte <= first_len.min(second_len));
                comparisons.push(byte);
                let matched = fingerprint_line_prefix(first.path(), line, byte).unwrap()
                    == fingerprint_line_prefix(second.path(), line, byte).unwrap();
                a.record_result(matched).unwrap();
                b.record_result(matched).unwrap();
            }
            ByteSearchStep::DifferenceAtByte { byte } => {
                assert_eq!(byte, expected_byte);
                return comparisons;
            }
        }
    }
    panic!("paired byte searches did not terminate");
}

#[test]
fn unequal_byte_lengths_only_compare_bytes_present_in_both_lines() {
    let long = vec![b'a'; 220];
    for (short, expected_byte, comparisons) in [
        (&b""[..], 1, vec![]),
        (&b"a"[..], 2, vec![1]),
        (&b"aa"[..], 3, vec![2]),
        (&b"Xa"[..], 1, vec![2, 1]),
        (&b"aX"[..], 2, vec![2, 1]),
        (&b"a\n"[..], 2, vec![2, 1]),
    ] {
        for (first, second) in [(short, long.as_slice()), (long.as_slice(), short)] {
            assert_eq!(
                paired_byte_search(first, second, 1, expected_byte),
                comparisons
            );
        }
    }
}

#[test]
fn paired_instances_find_first_differing_byte_after_line_search() {
    for (a, b, line, byte) in [
        (&b"abc\n"[..], &b"Xbc\n"[..], 1, 1),
        (&b"abc\n"[..], &b"abX\n"[..], 1, 3),
        (&b"abc\n"[..], &b"abXc\n"[..], 1, 3),
        (&b"abc"[..], &b"abcd"[..], 1, 4),
        (&b"abc"[..], &b"abc\n"[..], 1, 4),
        (&b"abc\nnext\n"[..], &b"abc\r\nnext\n"[..], 1, 4),
        (&b"\n"[..], &b"\r\n"[..], 1, 1),
        (&b"same\n"[..], &b"same\nadded\n"[..], 2, 1),
        (&b""[..], &b"nonempty"[..], 1, 1),
        (&b"a\nsame\n"[..], &b"a\n\xffame\n"[..], 2, 1),
        (&b"a\n\xff\x80"[..], &b"a\n\xff\x81"[..], 2, 2),
        ("café".as_bytes(), "cafè".as_bytes(), 1, 5),
        ("😀".as_bytes(), "😁".as_bytes(), 1, 4),
        ("e\u{301}".as_bytes(), "e\u{300}".as_bytes(), 1, 3),
    ] {
        paired_byte_search(a, b, line, byte);
        paired_byte_search(b, a, line, byte);
    }
    let mut a = b"same\n".to_vec();
    a.extend(vec![b'x'; 20_000]);
    let mut b = a.clone();
    b[16_390] = b'y';
    paired_byte_search(&a, &b, 2, 16_386);
}

#[test]
fn character_positions_validate_whole_lines_and_count_code_points() {
    let fixture = Fixture::new("earlier\ncafé😀e\u{301}\r\nlast".as_bytes());
    for (byte, character) in [
        (1, 1),
        (4, 4),
        (5, 4),
        (6, 5),
        (9, 5),
        (10, 6),
        (11, 7),
        (12, 7),
        (13, 8),
        (14, 9),
        (15, 10),
    ] {
        assert_eq!(
            utf8_character_position(fixture.path(), 2, byte).unwrap(),
            Some(character)
        );
    }
    assert_eq!(
        utf8_character_position(fixture.path(), 3, 5).unwrap(),
        Some(5)
    );
    assert_eq!(utf8_character_position(fixture.path(), 4, 1).unwrap(), None);
    for bytes in [
        &b""[..],
        &b"\xff"[..],
        &b"a\xff"[..],
        &b"\xc3"[..],
        &b"\xc0\x80"[..],
        &b"\xed\xa0\x80"[..],
        &b"\xf4\x90\x80\x80"[..],
        &b"\xc3\n"[..],
    ] {
        let fixture = Fixture::new(bytes);
        assert_eq!(utf8_character_position(fixture.path(), 1, 1).unwrap(), None);
        assert_eq!(
            utf8_character_position(fixture.path(), 1, bytes.len() as u64 + 1).unwrap(),
            None
        );
    }
    // The character starts at the end of the read buffer and finishes in the next.
    let mut bytes = vec![b'a'; 8191];
    bytes.extend("😀\nnext".as_bytes());
    let fixture = Fixture::new(&bytes);
    for byte in 8192..=8195 {
        assert_eq!(
            utf8_character_position(fixture.path(), 1, byte).unwrap(),
            Some(8192)
        );
    }
    assert_eq!(
        inspect_line(fixture.path(), 1).unwrap().unwrap().byte_len,
        8196
    );
    assert_hash(
        fingerprint_line_prefix(fixture.path(), 1, 8193).unwrap(),
        &bytes[..8193],
    );
    assert_hash(
        fingerprint_line_prefix(fixture.path(), 1, u64::MAX).unwrap(),
        &bytes[..8196],
    );
}
