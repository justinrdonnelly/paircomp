use super::*;

struct InterruptedReader<'a> {
    bytes: &'a [u8],
    interrupted: bool,
}

impl Read for InterruptedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.interrupted = !self.interrupted;
        if self.interrupted {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            self.bytes.read(&mut buffer[..1])
        }
    }
}

#[test]
fn short_and_interrupted_reads_preserve_boundaries_and_utf8() {
    let reader = || InterruptedReader {
        bytes: "a\ncafé\nlast".as_bytes(),
        interrupted: false,
    };
    let mut bytes = Vec::new();
    assert_eq!(
        scan_line(reader(), 2, u64::MAX, |chunk| bytes
            .extend_from_slice(chunk))
        .unwrap(),
        6
    );
    assert_eq!(bytes, "café\n".as_bytes());
    assert_eq!(character_position(reader(), 2, 5).unwrap(), Some(4));
    assert_eq!(character_position(reader(), 3, 5).unwrap(), Some(5));
}

#[test]
fn line_reads_return_io_errors() {
    struct FailingReader;
    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::PermissionDenied.into())
        }
    }
    assert!(
        matches!(scan_line(FailingReader, 2, 3, |_| {}), Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(
        matches!(character_position(FailingReader, 1, 1), Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}
