use super::*;

struct InterruptedReader<'a> {
    bytes: &'a [u8],
    calls: u8,
}

impl Read for InterruptedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        match self.calls {
            1 | 3 => Err(io::ErrorKind::Interrupted.into()),
            2 => self.bytes.read(&mut buffer[..2]),
            _ => self.bytes.read(buffer),
        }
    }
}

#[test]
fn interrupted_reads_preserve_full_and_prefix_scans() {
    let bytes = b"a\nb\nc\n";
    let reader = || InterruptedReader { bytes, calls: 0 };

    let full = scan_reader(reader(), None).unwrap();
    assert_eq!(full.byte_len, 6);
    assert_eq!(full.line_count, 3);
    assert_eq!(full.fingerprint.as_bytes(), blake3::hash(bytes).as_bytes());

    let prefix = scan_reader(reader(), Some(2)).unwrap();
    assert_eq!(prefix.byte_len, 4);
    assert_eq!(prefix.line_count, 2);
    assert_eq!(
        prefix.fingerprint.as_bytes(),
        blake3::hash(b"a\nb\n").as_bytes()
    );
}

#[test]
fn other_read_errors_are_returned() {
    struct FailingReader(u8);

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;
            match self.0 {
                1 => (&b"a\n"[..]).read(buffer),
                2 => Err(io::ErrorKind::Interrupted.into()),
                _ => Err(io::ErrorKind::PermissionDenied.into()),
            }
        }
    }

    assert!(matches!(
        scan_reader(FailingReader(0), None),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied
    ));
}
