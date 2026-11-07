use super::*;
use std::io::Cursor;

#[derive(Clone, Copy)]
struct Field {
    tag: u16,
    kind: u16,
    count: u64,
    value: u64,
}
fn number(out: &mut Vec<u8>, value: u64, width: usize, little: bool) {
    let bytes = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    out.extend_from_slice(if little {
        &bytes[..width]
    } else {
        &bytes[8 - width..]
    });
}
fn make(layout: Layout, fields: &[Field], payload_bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(if layout.little { b"II" } else { b"MM" });
    number(&mut out, if layout.big { 43 } else { 42 }, 2, layout.little);
    if layout.big {
        number(&mut out, 8, 2, layout.little);
        number(&mut out, 0, 2, layout.little);
    }
    number(
        &mut out,
        layout.header_len() as u64,
        layout.offset_len(),
        layout.little,
    );
    number(
        &mut out,
        fields.len() as u64,
        layout.count_len(),
        layout.little,
    );
    let data_start = layout.header_len()
        + layout.count_len()
        + fields.len() * layout.entry_len()
        + layout.offset_len();
    for field in fields {
        number(&mut out, u64::from(field.tag), 2, layout.little);
        number(&mut out, u64::from(field.kind), 2, layout.little);
        number(
            &mut out,
            field.count,
            if layout.big { 8 } else { 4 },
            layout.little,
        );
        let width = type_width(u64::from(field.kind)).unwrap_or(1);
        let bytes = field.count.saturating_mul(width);
        if field.count == 1 && bytes <= layout.offset_len() as u64 {
            let start = out.len();
            number(&mut out, field.value, width as usize, layout.little);
            out.resize(start + layout.offset_len(), 0);
        } else if bytes <= layout.offset_len() as u64 {
            out.resize(out.len() + layout.offset_len(), 0);
        } else {
            number(
                &mut out,
                data_start as u64,
                layout.offset_len(),
                layout.little,
            );
        }
    }
    number(&mut out, 0, layout.offset_len(), layout.little);
    out.resize(out.len() + payload_bytes, 0);
    out
}
fn layouts() -> [Layout; 4] {
    [
        Layout {
            little: true,
            big: false,
        },
        Layout {
            little: false,
            big: false,
        },
        Layout {
            little: true,
            big: true,
        },
        Layout {
            little: false,
            big: true,
        },
    ]
}
fn field(count: u64, kind: u16) -> Field {
    Field {
        tag: 347,
        kind,
        count,
        value: 0,
    }
}
fn check(bytes: Vec<u8>) -> io::Result<GuardedReader<Cursor<Vec<u8>>>> {
    admit(Cursor::new(bytes))
}
fn rejected(bytes: Vec<u8>, text: &str) {
    let error = match check(bytes) {
        Ok(_) => panic!("unexpected admission"),
        Err(e) => e,
    };
    assert!(error.to_string().contains(text), "{error:?}");
}
#[test]
fn tiff_guard_all_layouts_per_field_lower_equal_upper_and_widths() {
    for layout in layouts() {
        // Includes ASCII, signed and rational types, LONG8 and IFD8.
        for kind in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18] {
            let width = type_width(kind).unwrap();
            for count in [0, 1, MAX_VALUES_PER_ENTRY - 1, MAX_VALUES_PER_ENTRY] {
                assert!(check(make(
                    layout,
                    &[field(count, kind as u16)],
                    (count * width) as usize
                ))
                .is_ok());
            }
            rejected(
                make(layout, &[field(MAX_VALUES_PER_ENTRY + 1, kind as u16)], 0),
                "zu viele Werte in einem",
            );
        }
    }
}
#[test]
fn tiff_guard_all_layouts_entry_and_total_value_bounds_include_duplicates() {
    for layout in layouts() {
        for entries in [0, 1, MAX_ENTRIES - 1, MAX_ENTRIES] {
            let fields = vec![field(0, 7); entries as usize]; // duplicate tags count too
            assert!(check(make(layout, &fields, 0)).is_ok());
        }
        rejected(
            make(layout, &vec![field(0, 7); MAX_ENTRIES as usize + 1], 0),
            "zu viele IFD-Einträge",
        );
        for total in [MAX_VALUES_TOTAL - 1, MAX_VALUES_TOTAL] {
            let mut fields = vec![field(MAX_VALUES_PER_ENTRY, 7); 3];
            fields.push(field(total - 3 * MAX_VALUES_PER_ENTRY, 7));
            assert!(check(make(layout, &fields, MAX_VALUES_PER_ENTRY as usize)).is_ok());
        }
        let mut fields = vec![field(MAX_VALUES_PER_ENTRY, 7); 4];
        fields.push(field(1, 7));
        rejected(
            make(layout, &fields, MAX_VALUES_PER_ENTRY as usize),
            "zu viele IFD-Werte insgesamt",
        );
    }
}
#[test]
fn tiff_guard_all_layouts_unknown_types_truncation_and_declared_payload() {
    for layout in layouts() {
        rejected(
            make(layout, &[field(1, 14), field(1, 7)], 0),
            "unbekannter TIFF-Feldtyp",
        );
        let bytes = make(layout, &[field(16, 7)], 16);
        let envelope_len =
            layout.header_len() + layout.count_len() + layout.entry_len() + layout.offset_len();
        for cut in 0..envelope_len {
            assert!(check(bytes[..cut].to_vec()).is_err(), "cut={cut}");
        }
        rejected(bytes[..bytes.len() - 1].to_vec(), "außerhalb der Quelle");
    }
}
#[test]
fn tiff_guard_bigtiff_header_offset_overflow_and_overlap() {
    for layout in layouts() {
        if layout.big {
            let mut bytes = make(layout, &[], 0);
            bytes[4] = 0;
            bytes[5] = 0;
            rejected(bytes, "BigTIFF-Header");
            let mut bytes = make(layout, &[], 0);
            bytes[6] = 1;
            rejected(bytes, "BigTIFF-Header");
            let mut bytes = make(layout, &[], 0);
            bytes[8..16].fill(255);
            rejected(bytes, "läuft über");
        }
        let mut bytes = make(layout, &[], 0);
        let start = if layout.big { 8 } else { 4 };
        bytes[start..start + layout.offset_len()].fill(0);
        rejected(bytes, "überlappt");
    }
}
#[test]
fn tiff_guard_replays_checked_envelope_even_if_source_changes() {
    for layout in layouts() {
        let bytes = make(layout, &[field(1, 7)], 1);
        let mut guarded = check(bytes.clone()).unwrap();
        guarded.source.get_mut().fill(255);
        let envelope_len = bytes.len() - 1;
        let mut replay = vec![0; envelope_len];
        guarded.read_exact(&mut replay).unwrap();
        assert_eq!(replay, bytes[..envelope_len]);
        guarded.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(
            guarded.seek(SeekFrom::Current(-1)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            guarded.seek(SeekFrom::End(-1)).unwrap(),
            envelope_len as u64
        );
        let mut payload = [0];
        guarded.read_exact(&mut payload).unwrap();
        assert_eq!(payload, [255]); // whole-file immutability deliberately not promised
    }
}
fn valid_gray(layout: Layout) -> Vec<u8> {
    let mut fields = vec![
        Field {
            tag: 256,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 257,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 258,
            kind: 3,
            count: 1,
            value: 8,
        },
        Field {
            tag: 259,
            kind: 3,
            count: 1,
            value: 1,
        },
        Field {
            tag: 262,
            kind: 3,
            count: 1,
            value: 1,
        },
        Field {
            tag: 273,
            kind: 4,
            count: 1,
            value: 0,
        },
        Field {
            tag: 277,
            kind: 3,
            count: 1,
            value: 1,
        },
        Field {
            tag: 278,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 279,
            kind: 4,
            count: 1,
            value: 1,
        },
    ];
    fields[5].value = (layout.header_len()
        + layout.count_len()
        + fields.len() * layout.entry_len()
        + layout.offset_len()) as u64;
    let mut bytes = make(layout, &fields, 1);
    *bytes.last_mut().unwrap() = 73;
    bytes
}
#[test]
fn tiff_guard_valid_import_recognized_classic_layouts_and_budget_release() {
    use crate::resource_budget::ResourceBudget;
    for layout in layouts().into_iter().filter(|layout| !layout.big) {
        let bytes = valid_gray(layout);
        let image = crate::image_io::decode(&bytes).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.pixels, [73, 73, 73, 255]);
        let budget = ResourceBudget::new(32 * 1024 * 1024, 0).unwrap();
        let image = crate::image_io::decode_with_budget(&bytes, &budget).unwrap();
        assert_eq!(image.image.pixels, [73, 73, 73, 255]);
        drop(image);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
}
#[test]
fn tiff_guard_large_jpeg_tables_regression_rejected_before_constructor() {
    for layout in layouts() {
        for payload in [0, 1024 * 1024] {
            let bytes = make(layout, &[field(1024 * 1024, 7)], payload);
            rejected(bytes.clone(), "zu viele Werte in einem");
            // image 0.25.10 does not guess BigTIFF from its signature.
            // BigTIFF is exercised above at the guard boundary only.
            if !layout.big {
                let error = crate::image_io::decode(&bytes).unwrap_err();
                assert!(error.contains("TIFF-Konstruktorgrenze"), "{error}");
                assert!(error.contains("zu viele Werte in einem"), "{error}");
            }
        }
    }
}
#[test]
fn tiff_guard_typed_payload_offset_overflow_is_checked() {
    for layout in layouts() {
        let mut bytes = make(layout, &[field(16, 7)], 16);
        let offset_at = layout.header_len() + layout.count_len() + if layout.big { 12 } else { 8 };
        bytes[offset_at..offset_at + layout.offset_len()].fill(255);
        rejected(
            bytes,
            if layout.big {
                "läuft über"
            } else {
                "außerhalb der Quelle"
            },
        );
    }
}
#[test]
fn tiff_guard_replay_caps_live_reads_at_directory_boundary() {
    for layout in layouts() {
        let mut bytes = make(layout, &[field(0, 7)], 0);
        let old_start = layout.header_len();
        let mut new_pointer = Vec::new();
        number(
            &mut new_pointer,
            (old_start + 7) as u64,
            layout.offset_len(),
            layout.little,
        );
        let pointer_at = if layout.big { 8 } else { 4 };
        bytes[pointer_at..pointer_at + layout.offset_len()].copy_from_slice(&new_pointer);
        bytes.splice(old_start..old_start, [19; 7]);
        let mut guarded = check(bytes.clone()).unwrap();
        guarded.source.get_mut().fill(255);
        let mut replay = vec![0; bytes.len()];
        guarded.read_exact(&mut replay).unwrap();
        assert_eq!(&replay[..old_start], &bytes[..old_start]);
        assert_eq!(&replay[old_start..old_start + 7], &[255; 7]);
        assert_eq!(&replay[old_start + 7..], &bytes[old_start + 7..]);
    }
}
// Exact classic little-endian structure used by the original memory probe.
// Generate the payload; never embed a megabyte of binary repository data.
fn measured_jpeg_fixture(full_payload: bool) -> Vec<u8> {
    let layout = Layout {
        little: true,
        big: false,
    };
    let count = 1024 * 1024;
    let fields = [
        Field {
            tag: 256,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 257,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 258,
            kind: 3,
            count: 1,
            value: 8,
        },
        Field {
            tag: 259,
            kind: 3,
            count: 1,
            value: 7,
        },
        Field {
            tag: 262,
            kind: 3,
            count: 1,
            value: 1,
        },
        Field {
            tag: 273,
            kind: 4,
            count: 1,
            value: 134 + count,
        },
        Field {
            tag: 277,
            kind: 3,
            count: 1,
            value: 1,
        },
        Field {
            tag: 278,
            kind: 4,
            count: 1,
            value: 1,
        },
        Field {
            tag: 279,
            kind: 4,
            count: 1,
            value: 1,
        },
        field(count, 7),
    ];
    let mut bytes = make(
        layout,
        &fields,
        if full_payload { count as usize + 1 } else { 0 },
    );
    if full_payload {
        bytes[134..136].copy_from_slice(&[255, 216]);
        bytes[134 + count as usize - 2..134 + count as usize].copy_from_slice(&[255, 217]);
    }
    bytes
}
#[test]
fn tiff_guard_measured_jpeg_table_fixtures_reject_and_release_budget() {
    use crate::resource_budget::ResourceBudget;
    for full in [false, true] {
        let fixture = measured_jpeg_fixture(full);
        assert_eq!(fixture.len(), if full { 1_048_711 } else { 134 });
        let budget = ResourceBudget::new(16 * 1024 * 1024, 0).unwrap();
        let error = crate::image_io::decode_with_budget(&fixture, &budget).unwrap_err();
        assert!(
            error.contains("zu viele Werte in einem IFD-Feld"),
            "{error}"
        );
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
}
#[test]
fn tiff_guard_bigtiff_valid_envelope_is_replayed_without_format_guess_claim() {
    for layout in layouts().into_iter().filter(|layout| layout.big) {
        let bytes = valid_gray(layout);
        let mut guarded = check(bytes.clone()).unwrap();
        let mut replay = Vec::new();
        guarded.read_to_end(&mut replay).unwrap();
        assert_eq!(replay, bytes);
    }
}
struct FailingSource {
    bytes: Cursor<Vec<u8>>,
    read_error: bool,
    seek_error: bool,
}
impl Read for FailingSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.read_error {
            return Err(io::Error::other("injected read failure"));
        }
        self.bytes.read(out)
    }
}
impl Seek for FailingSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        if self.seek_error {
            return Err(io::Error::other("injected seek failure"));
        }
        self.bytes.seek(from)
    }
}
#[test]
fn tiff_guard_source_read_seek_errors_are_propagated() {
    let bytes = make(
        Layout {
            little: true,
            big: false,
        },
        &[field(1, 7)],
        1,
    );
    for (read_error, seek_error, message) in [
        (true, false, "injected read failure"),
        (false, true, "injected seek failure"),
    ] {
        let source = FailingSource {
            bytes: Cursor::new(bytes.clone()),
            read_error,
            seek_error,
        };
        assert!(matches!(admit(source),Err(error) if error.to_string().contains(message)));
    }
    let source = FailingSource {
        bytes: Cursor::new(bytes.clone()),
        read_error: false,
        seek_error: false,
    };
    let mut guarded = admit(source).unwrap();
    guarded
        .seek(SeekFrom::Start(bytes.len() as u64 - 1))
        .unwrap();
    guarded.source.read_error = true;
    assert!(guarded
        .read(&mut [0])
        .unwrap_err()
        .to_string()
        .contains("injected read failure"));
    guarded.source.seek_error = true;
    assert!(guarded
        .seek(SeekFrom::Start(0))
        .unwrap_err()
        .to_string()
        .contains("injected seek failure"));
}
#[test]
fn tiff_guard_exact_payload_end_and_zero_length_read_boundaries() {
    for layout in layouts() {
        let bytes = make(layout, &[field(16, 7)], 16);
        let mut guarded = check(bytes.clone()).unwrap();
        assert_eq!(guarded.read(&mut []).unwrap(), 0);
        assert_eq!(guarded.stream_position().unwrap(), 0);
        guarded.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(guarded.read(&mut [0]).unwrap(), 0);
        rejected(bytes[..bytes.len() - 1].to_vec(), "außerhalb der Quelle");
    }
}
