//! A seekable reader that never exposes bytes at offsets beyond a fixed limit.
//!
//! The limit is an offset bound, not a cumulative I/O budget: decoders may seek
//! backwards and reread bytes. Reads ending exactly at the bound are followed
//! by a one-byte probe so a source that grew after construction is reported as
//! oversized instead of being silently treated as a valid truncated stream.

use std::io::{self, Read, Seek, SeekFrom};

/// `Read + Seek` adapter that restricts the visible stream to `max_offset`.
pub(crate) struct BoundedReader<R> {
    inner: R,
    max_offset: u64,
    position: u64,
}

impl<R: Seek> BoundedReader<R> {
    /// Wrap `inner` at its current position. Construction fails if that
    /// position is already outside the permitted range.
    pub(crate) fn new(mut inner: R, max_offset: u64) -> io::Result<Self> {
        let position = inner.stream_position()?;
        if position > max_offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reader starts past its byte limit",
            ));
        }
        Ok(Self {
            inner,
            max_offset,
            position,
        })
    }

    fn restore(&mut self, position: u64, error: io::Error) -> io::Error {
        if self.inner.seek(SeekFrom::Start(position)).is_ok() {
            self.position = position;
            error
        } else {
            io::Error::other(format!("{error}; failed to restore reader position"))
        }
    }
}

impl<R: Read + Seek> Read for BoundedReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }

        let remaining = self.max_offset - self.position;
        if remaining == 0 {
            let mut probe = [0u8; 1];
            return match read_retrying_interrupts(&mut self.inner, &mut probe)? {
                0 => Ok(0),
                _ => {
                    let error = io::Error::new(
                        io::ErrorKind::InvalidData,
                        "input exceeds the configured byte limit",
                    );
                    Err(self.restore(self.position, error))
                }
            };
        }

        let allowed = usize::try_from(remaining.min(output.len() as u64))
            .expect("the value is bounded by output.len()");
        let count = read_retrying_interrupts(&mut self.inner, &mut output[..allowed])?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reader offset overflow"))?;
        Ok(count)
    }
}

fn read_retrying_interrupts(reader: &mut impl Read, output: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(output) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

impl<R: Read + Seek> Seek for BoundedReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let old_position = self.position;
        let target = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => match add_signed(old_position, offset) {
                Some(target) => target,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek is before the start of input",
                    ));
                }
            },
            SeekFrom::End(offset) => {
                let end = match self.inner.seek(SeekFrom::End(0)) {
                    Ok(end) => end,
                    Err(error) => return Err(self.restore(old_position, error)),
                };
                if let Err(error) = self.inner.seek(SeekFrom::Start(old_position)) {
                    self.position = old_position;
                    return Err(error);
                }
                if end > self.max_offset {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "input exceeds the configured byte limit",
                    ));
                }
                match add_signed(end, offset) {
                    Some(target) => target,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "seek is before the start of input",
                        ));
                    }
                }
            }
        };

        if target > self.max_offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek is past the configured byte limit",
            ));
        }
        match self.inner.seek(SeekFrom::Start(target)) {
            Ok(actual) if actual == target => {
                self.position = actual;
                Ok(actual)
            }
            Ok(_) => Err(self.restore(
                old_position,
                io::Error::new(io::ErrorKind::InvalidInput, "reader seek was not exact"),
            )),
            Err(error) => Err(self.restore(old_position, error)),
        }
    }
}

fn add_signed(base: u64, offset: i64) -> Option<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedReader;
    use std::{
        cell::RefCell,
        io::{self, Cursor, Read, Seek, SeekFrom},
        rc::Rc,
    };

    #[test]
    fn accepts_exact_limit_and_reports_oversize_with_probe() {
        let mut exact = BoundedReader::new(Cursor::new(&b"abc"[..]), 3).unwrap();
        let mut output = [0; 8];
        assert_eq!(exact.read(&mut output).unwrap(), 3);
        assert_eq!(&output[..3], b"abc");
        assert_eq!(exact.read(&mut output).unwrap(), 0);

        let mut oversized = BoundedReader::new(Cursor::new(&b"abcd"[..]), 3).unwrap();
        assert_eq!(oversized.read(&mut output).unwrap(), 3);
        assert_eq!(
            oversized.read(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(oversized.seek(SeekFrom::Start(0)).unwrap(), 0);
        assert_eq!(oversized.read(&mut output).unwrap(), 3);
        assert_eq!(
            oversized.read(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn seeks_are_checked_and_failed_seeks_preserve_position() {
        let mut reader = BoundedReader::new(Cursor::new(&b"abcd"[..]), 4).unwrap();
        assert_eq!(reader.seek(SeekFrom::Start(2)).unwrap(), 2);
        assert_eq!(reader.seek(SeekFrom::Current(2)).unwrap(), 4);
        assert_eq!(reader.seek(SeekFrom::Current(-3)).unwrap(), 1);
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), 4);
        for invalid in [
            SeekFrom::Start(5),
            SeekFrom::Current(1),
            SeekFrom::Current(-5),
            SeekFrom::End(1),
            SeekFrom::End(-5),
        ] {
            assert_eq!(
                reader.seek(invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(reader.stream_position().unwrap(), 4);
        }
        assert_eq!(
            BoundedReader::new(Cursor::new(&b"abc"[..]), 2)
                .unwrap()
                .seek(SeekFrom::End(0))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rereading_bytes_does_not_consume_a_cumulative_budget() {
        let mut reader = BoundedReader::new(Cursor::new(&b"abcd"[..]), 4).unwrap();
        let mut output = [0; 4];
        for _ in 0..3 {
            reader.seek(SeekFrom::Start(0)).unwrap();
            assert_eq!(reader.read(&mut output).unwrap(), 4);
            assert_eq!(&output, b"abcd");
        }
        assert_eq!(reader.read(&mut output).unwrap(), 0);
    }

    #[test]
    fn interrupted_reads_are_retried() {
        struct InterruptOnce(Cursor<Vec<u8>>, bool);
        impl Read for InterruptOnce {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if !self.1 {
                    self.1 = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.0.read(output)
            }
        }
        impl Seek for InterruptOnce {
            fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
                self.0.seek(from)
            }
        }
        let mut reader =
            BoundedReader::new(InterruptOnce(Cursor::new(b"ok".to_vec()), false), 2).unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"ok");
    }

    #[test]
    fn interrupted_limit_probe_is_retried() {
        struct InterruptProbe(Cursor<Vec<u8>>, usize);
        impl Read for InterruptProbe {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.1 += 1;
                if self.1 == 2 {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.0.read(output)
            }
        }
        impl Seek for InterruptProbe {
            fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
                self.0.seek(from)
            }
        }
        let mut reader =
            BoundedReader::new(InterruptProbe(Cursor::new(b"ok".to_vec()), 0), 2).unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"ok");
    }

    #[derive(Clone)]
    struct SharedSource(Rc<RefCell<(Vec<u8>, u64)>>);

    impl Read for SharedSource {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let source = self.0.borrow();
            let (bytes, position) = (&source.0, source.1 as usize);
            let count = output.len().min(bytes.len().saturating_sub(position));
            output[..count].copy_from_slice(&bytes[position..position + count]);
            drop(source);
            self.0.borrow_mut().1 += count as u64;
            Ok(count)
        }
    }

    impl Seek for SharedSource {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            let mut source = self.0.borrow_mut();
            let current = source.1;
            let end = source.0.len() as u64;
            let target = match from {
                SeekFrom::Start(offset) => Some(offset),
                SeekFrom::Current(offset) => super::add_signed(current, offset),
                SeekFrom::End(offset) => super::add_signed(end, offset),
            }
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid test seek"))?;
            source.1 = target;
            Ok(target)
        }
    }

    #[test]
    fn detects_source_growth_after_reaching_the_limit() {
        let source = Rc::new(RefCell::new((b"abc".to_vec(), 0)));
        let mut reader = BoundedReader::new(SharedSource(Rc::clone(&source)), 3).unwrap();
        let mut output = [0; 4];
        assert_eq!(reader.read(&mut output).unwrap(), 3);
        source.borrow_mut().0.push(b'd');
        assert_eq!(
            reader.read(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
