//! Conservative first-IFD policy for image 0.25.10 / tiff 0.11.3 only.
//!
//! This bounds declarations responsible for observed early Vec<Value> growth.
//! It does not establish an allocator/RSS bound or validate TIFF raster data.
//! Both the header and the first IFD are replayed from the checked snapshot so
//! the decoder cannot reinterpret changed declaration bytes after this check.
//! Other source bytes remain live; this is not a whole-file immutable snapshot.
use std::io::{self, Read, Seek, SeekFrom};

const MAX_ENTRIES: u64 = 1024;
const MAX_VALUES_PER_ENTRY: u64 = 65_536;
const MAX_VALUES_TOTAL: u64 = 262_144;

fn reject(reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("TIFF-Konstruktorgrenze: {reason}"),
    )
}

#[derive(Clone, Copy)]
struct Layout {
    little: bool,
    big: bool,
}
impl Layout {
    fn number(self, bytes: &[u8]) -> u64 {
        if self.little {
            bytes
                .iter()
                .enumerate()
                .fold(0, |n, (i, b)| n | (u64::from(*b) << (i * 8)))
        } else {
            bytes.iter().fold(0, |n, b| (n << 8) | u64::from(*b))
        }
    }
    fn header_len(self) -> usize {
        if self.big {
            16
        } else {
            8
        }
    }
    fn count_len(self) -> usize {
        if self.big {
            8
        } else {
            2
        }
    }
    fn entry_len(self) -> usize {
        if self.big {
            20
        } else {
            12
        }
    }
    fn offset_len(self) -> usize {
        if self.big {
            8
        } else {
            4
        }
    }
}

fn type_width(kind: u64) -> Option<u64> {
    match kind {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 | 13 => Some(4),
        5 | 10 | 12 | 16 | 17 | 18 => Some(8),
        _ => None,
    }
}

/// Only the checked structural ranges are frozen. No arbitrary payload copy.
pub(super) struct GuardedReader<R> {
    source: R,
    header: Vec<u8>,
    directory: Vec<u8>,
    directory_start: u64,
    directory_end: u64,
    position: u64,
}

pub(super) fn admit<R: Read + Seek>(mut source: R) -> io::Result<GuardedReader<R>> {
    source.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 16];
    source.read_exact(&mut header[..8])?;
    let little = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(reject("ungültige Byteordnung")),
    };
    let mut layout = Layout { little, big: false };
    match layout.number(&header[2..4]) {
        42 => {}
        43 => {
            layout.big = true;
            source.read_exact(&mut header[8..16])?;
            if layout.number(&header[4..6]) != 8 || layout.number(&header[6..8]) != 0 {
                return Err(reject("nicht unterstützter BigTIFF-Header"));
            }
        }
        _ => return Err(reject("nicht unterstützte TIFF-Version")),
    }
    let directory_start = if layout.big {
        layout.number(&header[8..16])
    } else {
        layout.number(&header[4..8])
    };
    // Deliberately reject overlapping header/IFD ranges. They cannot have
    // ambiguous precedence in the replay reader, even for malformed input.
    if directory_start < layout.header_len() as u64 {
        return Err(reject("erste IFD überlappt den Header"));
    }
    let source_len = source.seek(SeekFrom::End(0))?;
    let count_end = directory_start
        .checked_add(layout.count_len() as u64)
        .ok_or_else(|| reject("IFD-Offset läuft über"))?;
    if count_end > source_len {
        return Err(reject("abgeschnittene IFD-Anzahl"));
    }
    source.seek(SeekFrom::Start(directory_start))?;
    let mut count_bytes = [0u8; 8];
    source.read_exact(&mut count_bytes[..layout.count_len()])?;
    let entries = layout.number(&count_bytes[..layout.count_len()]);
    if entries > MAX_ENTRIES {
        return Err(reject("zu viele IFD-Einträge"));
    }
    let directory_len = entries
        .checked_mul(layout.entry_len() as u64)
        .and_then(|n| n.checked_add(layout.count_len() as u64))
        .and_then(|n| n.checked_add(layout.offset_len() as u64))
        .ok_or_else(|| reject("IFD-Länge läuft über"))?;
    let directory_end = directory_start
        .checked_add(directory_len)
        .ok_or_else(|| reject("IFD-Ende läuft über"))?;
    if directory_end > source_len {
        return Err(reject("abgeschnittene erste IFD"));
    }
    let directory_len =
        usize::try_from(directory_len).map_err(|_| reject("IFD-Länge nicht darstellbar"))?;
    let mut directory = Vec::new();
    directory
        .try_reserve_exact(directory_len)
        .map_err(|_| reject("IFD-Puffer nicht verfügbar"))?;
    directory.resize(directory_len, 0);
    // Reuse the count bytes already checked; do not read them a second time.
    directory[..layout.count_len()].copy_from_slice(&count_bytes[..layout.count_len()]);
    source.read_exact(&mut directory[layout.count_len()..])?;
    let mut total_values = 0u64;
    for entry in directory[layout.count_len()..directory_len - layout.offset_len()]
        .chunks_exact(layout.entry_len())
    {
        let width = type_width(layout.number(&entry[2..4]))
            .ok_or_else(|| reject("unbekannter TIFF-Feldtyp"))?;
        // Count denotes elements, not payload bytes. Even UNDEFINED/BYTE
        // fields can expand into one Value per byte in this dependency.
        let count_end = if layout.big { 12 } else { 8 };
        let count = layout.number(&entry[4..count_end]);
        if count > MAX_VALUES_PER_ENTRY {
            return Err(reject("zu viele Werte in einem IFD-Feld"));
        }
        total_values = total_values
            .checked_add(count)
            .ok_or_else(|| reject("Wertanzahl läuft über"))?;
        if total_values > MAX_VALUES_TOTAL {
            return Err(reject("zu viele IFD-Werte insgesamt"));
        }
        let bytes = count
            .checked_mul(width)
            .ok_or_else(|| reject("Feldlänge läuft über"))?;
        if bytes > layout.offset_len() as u64 {
            let offset = layout.number(&entry[count_end..]);
            let end = offset
                .checked_add(bytes)
                .ok_or_else(|| reject("Feldende läuft über"))?;
            if end > source_len {
                return Err(reject("IFD-Feld liegt außerhalb der Quelle"));
            }
        }
    }
    Ok(GuardedReader {
        source,
        header: header[..layout.header_len()].to_vec(),
        directory,
        directory_start,
        directory_end,
        position: 0,
    })
}

impl<R: Read + Seek> Read for GuardedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let header_end = self.header.len() as u64;
        let cached = if self.position < header_end {
            Some(&self.header[self.position as usize..])
        } else if self.position >= self.directory_start && self.position < self.directory_end {
            Some(&self.directory[(self.position - self.directory_start) as usize..])
        } else {
            None
        };
        let count = if let Some(bytes) = cached {
            let count = out.len().min(bytes.len());
            out[..count].copy_from_slice(&bytes[..count]);
            count
        } else {
            let limit = if self.position < self.directory_start {
                out.len().min(
                    usize::try_from(self.directory_start - self.position).unwrap_or(usize::MAX),
                )
            } else {
                out.len()
            };
            self.source.seek(SeekFrom::Start(self.position))?;
            self.source.read(&mut out[..limit])?
        };
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| reject("Readerposition läuft über"))?;
        Ok(count)
    }
}

impl<R: Read + Seek> Seek for GuardedReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self
                .position
                .checked_add_signed(n)
                .ok_or_else(|| reject("ungültige relative Readerposition"))?,
            SeekFrom::End(n) => self
                .source
                .seek(SeekFrom::End(0))?
                .checked_add_signed(n)
                .ok_or_else(|| reject("ungültige Readerposition relativ zum Ende"))?,
        };
        // Preserve the underlying offset-bound contract even for cached ranges.
        self.source.seek(SeekFrom::Start(target))?;
        self.position = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests;
