//! Account the complete encoded copy made by image 0.25.10's JPEG constructor.
//!
//! This private adapter supports only that constructor's `read_to_end` operation.
//! Other input methods fail closed if an image update changes the constructor.
//! It sits outside BufReader so std's read_to_end growth cannot bypass it.
//!
//! The caller admits length plus slack before constructing the decoder and owns
//! the lease until the decoder is gone. We reserve fallibly once, check actual
//! Vec capacity, then fill without further allocation. An allocator may report
//! capacity above the request: that allocation necessarily precedes this check;
//! rejection drops it before any pixels are decoded. This is admission policy,
//! not a hard allocator/process ceiling or a bound on other codec scratch.
use crate::resource_budget::BudgetLease;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

const CAPACITY_SLACK: usize = 8192;

pub(super) fn copy_allowance(length: u64) -> Result<usize, String> {
    usize::try_from(length)
        .ok()
        .and_then(|length| length.checked_add(CAPACITY_SLACK))
        .ok_or_else(|| "Die kodierte Bildgröße läuft über.".into())
}

pub(super) struct JpegBudgetReader<'a, R> {
    inner: R,
    length: u64,
    lease: &'a mut BudgetLease,
}

impl<'a, R> JpegBudgetReader<'a, R> {
    pub(super) fn new(inner: R, length: u64, lease: &'a mut BudgetLease) -> Self {
        Self {
            inner,
            length,
            lease,
        }
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "JPEG constructor must use the admitted read_to_end operation",
    )
}

fn admit_capacity(lease: &mut BudgetLease, capacity: usize) -> io::Result<()> {
    if capacity > lease.bytes() {
        lease
            .try_resize(capacity)
            .map_err(|error| io::Error::other(super::import_budget_error(error)))?;
    }
    Ok(())
}

impl<R: Read> Read for JpegBudgetReader<'_, R> {
    fn read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    fn read_to_end(&mut self, output: &mut Vec<u8>) -> io::Result<usize> {
        // image's constructor always supplies an empty Vec. Refuse unexpected
        // callers instead of copying/reallocating an existing destination.
        if !output.is_empty() || output.capacity() != 0 {
            return Err(unsupported());
        }
        let length = usize::try_from(self.length)
            .map_err(|_| io::Error::other("Die kodierte Bildgröße läuft über."))?;
        // The production caller already admitted length plus slack before
        // into_decoder. Keep this guard for private callers and future changes.
        if length > self.lease.bytes() {
            return Err(io::Error::other("JPEG encoded input was not admitted"));
        }
        let mut input = Vec::new();
        input
            .try_reserve_exact(length)
            .map_err(|_| io::Error::other("Nicht genügend Speicher zum Lesen der Bilddatei."))?;
        admit_capacity(self.lease, input.capacity())?;
        let mut chunk = [0u8; 8192];
        loop {
            let remaining = length - input.len();
            // One probe byte distinguishes exact EOF from a file that grew
            // beyond the length admitted from the same open handle.
            let request = remaining.min(chunk.len() - 1) + 1;
            let count = match self.inner.read(&mut chunk[..request]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                let count = input.len();
                *output = input; // Hand the allocation to JPEG without copying.
                return Ok(count);
            }
            if count > remaining {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JPEG input exceeds its admitted byte limit",
                ));
            }
            // Capacity >= admitted length, so this cannot grow the Vec.
            input.extend_from_slice(&chunk[..count]);
        }
    }
}

impl<R: Read> BufRead for JpegBudgetReader<'_, R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(unsupported())
    }

    fn consume(&mut self, _amount: usize) {
        // fill_buf never returns a successful borrow to consume.
    }
}

impl<R> Seek for JpegBudgetReader<'_, R> {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    use std::io::Cursor;

    #[test]
    fn exact_length_shorter_source_and_empty_source_are_supported() {
        for (bytes, limit) in [(&b"abcd"[..], 4), (&b"ab"[..], 4), (&b""[..], 0)] {
            let allowance = copy_allowance(limit).unwrap();
            let budget = ResourceBudget::new(allowance, 0).unwrap();
            let mut lease = budget
                .try_acquire(allowance, AdmissionClass::Normal)
                .unwrap();
            let mut output = Vec::new();
            JpegBudgetReader::new(Cursor::new(bytes), limit, &mut lease)
                .read_to_end(&mut output)
                .unwrap();
            assert_eq!(output, bytes);
            assert!(lease.bytes() >= output.capacity());
            assert_eq!(budget.snapshot().used_bytes, lease.bytes());
            drop(output);
            drop(lease);
            assert_eq!(budget.snapshot().used_bytes, 0);
        }
    }

    #[test]
    fn source_growth_and_read_errors_drop_partial_copy() {
        struct FailsAfterOne(bool);
        impl Read for FailsAfterOne {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected read failure"));
                }
                self.0 = true;
                output[0] = 1;
                Ok(1)
            }
        }
        let allowance = copy_allowance(3).unwrap();
        let budget = ResourceBudget::new(allowance, 0).unwrap();
        let mut lease = budget
            .try_acquire(allowance, AdmissionClass::Normal)
            .unwrap();
        let mut output = Vec::new();
        struct GrowsAfterFirstRead {
            bytes: Cursor<Vec<u8>>,
            grew: bool,
        }
        impl Read for GrowsAfterFirstRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                let count = self.bytes.read(output)?;
                if !self.grew {
                    self.bytes.get_mut().push(b'd');
                    self.grew = true;
                }
                Ok(count)
            }
        }
        let source = GrowsAfterFirstRead {
            bytes: Cursor::new(b"abc".to_vec()),
            grew: false,
        };
        let error = JpegBudgetReader::new(source, 3, &mut lease)
            .read_to_end(&mut output)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(output.capacity(), 0);
        let error = JpegBudgetReader::new(FailsAfterOne(false), 3, &mut lease)
            .read_to_end(&mut output)
            .unwrap_err();
        assert!(error.to_string().contains("injected read failure"));
        assert_eq!(output.capacity(), 0);
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn interrupted_reads_including_eof_probe_are_retried() {
        struct Interrupting {
            bytes: Cursor<&'static [u8]>,
            interrupt: bool,
        }
        impl Read for Interrupting {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.interrupt = !self.interrupt;
                if self.interrupt {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.bytes.read(output)
            }
        }
        let allowance = copy_allowance(2).unwrap();
        let budget = ResourceBudget::new(allowance, 0).unwrap();
        let mut lease = budget
            .try_acquire(allowance, AdmissionClass::Normal)
            .unwrap();
        let mut output = Vec::new();
        JpegBudgetReader::new(
            Interrupting {
                bytes: Cursor::new(b"ok"),
                interrupt: false,
            },
            2,
            &mut lease,
        )
        .read_to_end(&mut output)
        .unwrap();
        assert_eq!(output, b"ok");
    }

    #[test]
    fn capacity_rejection_preserves_other_leases_and_overflow_is_checked() {
        let budget = ResourceBudget::new(10, 0).unwrap();
        let existing = budget.try_acquire(3, AdmissionClass::Normal).unwrap();
        let mut lease = budget.try_acquire(7, AdmissionClass::Normal).unwrap();
        assert!(admit_capacity(&mut lease, 8).is_err());
        assert_eq!(lease.bytes(), 7);
        assert_eq!(existing.bytes(), 3);
        assert_eq!(budget.snapshot().used_bytes, 10);
        assert!(copy_allowance(u64::MAX).is_err());
    }

    #[test]
    fn impossible_allocation_is_fallible_and_releases_the_lease() {
        // Exceeds Vec's isize::MAX layout bound on both 32- and 64-bit hosts;
        // try_reserve_exact rejects before requesting a real huge allocation.
        let length = usize::MAX - CAPACITY_SLACK;
        let allowance = copy_allowance(length as u64).unwrap();
        let budget = ResourceBudget::new(allowance, 0).unwrap();
        let mut lease = budget
            .try_acquire(allowance, AdmissionClass::Normal)
            .unwrap();
        let mut output = Vec::new();
        let error = JpegBudgetReader::new(Cursor::new(b""), length as u64, &mut lease)
            .read_to_end(&mut output)
            .unwrap_err();
        assert!(error.to_string().contains("Nicht genügend Speicher"));
        assert_eq!(output.capacity(), 0);
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn alternate_input_methods_and_unadmitted_input_fail_closed() {
        let budget = ResourceBudget::new(1, 0).unwrap();
        let mut lease = budget.try_acquire(1, AdmissionClass::Normal).unwrap();
        let mut reader = JpegBudgetReader::new(Cursor::new(b"ok"), 2, &mut lease);
        assert!(reader.read(&mut [0]).is_err());
        assert!(reader.fill_buf().is_err());
        assert!(reader.seek(SeekFrom::Start(0)).is_err());
        let mut output = Vec::new();
        assert!(reader.read_to_end(&mut output).is_err());
        assert_eq!(output.capacity(), 0);
    }
}
