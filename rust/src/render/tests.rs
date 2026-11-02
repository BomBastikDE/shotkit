    use super::*;
    #[cfg(windows)]
    use std::ptr::null_mut;

    #[test]
    fn rgb_is_bgr_encoding_for_gdi() {
        assert_eq!(rgb(Color::rgba(1, 2, 3, 255)), 0x00030201);
    }

    #[cfg(windows)]
    struct DibCanvas {
        dc: Hdc,
        bitmap: Hgdiobj,
        previous: Hgdiobj,
        bits: *mut u8,
        width: usize,
        height: usize,
    }

    #[cfg(windows)]
    impl DibCanvas {
        // SAFETY CONTRACT:
        // Voraussetzung: Maße sind positiv, passen in i32 und width*height*4 überläuft nicht; GDI-Arbeit ist synchronisiert.
        // Warum erfüllt bzw. offen: Aktuelle Tests nutzen feste kleine Maße; der Konstruktor selbst prüft diese Grenzen nicht vollständig.
        // Nachrichten: GDI-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
        unsafe fn new(width: usize, height: usize, bgra: [u8; 4]) -> Self {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null(), "CreateCompatibleDC failed");
            let byte_len = width * height * 4;
            let info = BitmapInfo {
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width: width as i32,
                    height: -(height as i32),
                    planes: 1,
                    bit_count: 32,
                    compression: 0,
                    image_size: byte_len as u32,
                    xppm: 0,
                    yppm: 0,
                    colors_used: 0,
                    colors_important: 0,
                },
                color: 0,
            };
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, 0, &mut bits, null_mut(), 0);
            assert!(
                !bitmap.is_null() && !bits.is_null(),
                "CreateDIBSection failed"
            );
            let previous = SelectObject(dc, bitmap);
            assert!(
                !previous.is_null() && previous as isize != -1,
                "SelectObject failed"
            );
            let mut canvas = Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            };
            assert_ne!(
                GdiFlush(),
                0,
                "GdiFlush before direct DIB initialization failed"
            );
            for pixel in canvas.bytes_mut().chunks_exact_mut(4) {
                pixel.copy_from_slice(&bgra);
            }
            canvas
        }

        fn bytes(&self) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.bits, self.width * self.height * 4) }
        }
        fn bytes_mut(&mut self) -> &mut [u8] {
            unsafe { std::slice::from_raw_parts_mut(self.bits, self.width * self.height * 4) }
        }
        fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
            let i = (y * self.width + x) * 4;
            self.bytes()[i..i + 4].try_into().unwrap()
        }
        fn changed_from(&self, bgra: [u8; 4]) -> bool {
            self.bytes().chunks_exact(4).any(|pixel| pixel != bgra)
        }
    }

    #[cfg(windows)]
    impl Drop for DibCanvas {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn gdi_shapes_and_blur_change_dib_pixels_without_a_window_or_display() {
        let background = [24, 32, 48, 255];
        unsafe {
            let cases = [
                Shape::new(ShapeKind::Line {
                    start: Point::new(5.0, 5.0),
                    end: Point::new(58.0, 42.0),
                }),
                Shape::new(ShapeKind::Arrow {
                    start: Point::new(6.0, 48.0),
                    end: Point::new(58.0, 8.0),
                }),
                Shape::new(ShapeKind::Rectangle {
                    start: Point::new(8.0, 8.0),
                    end: Point::new(54.0, 52.0),
                }),
                Shape::new(ShapeKind::Ellipse {
                    start: Point::new(8.0, 8.0),
                    end: Point::new(54.0, 52.0),
                }),
            ];
            for shape in cases {
                let canvas = DibCanvas::new(64, 64, background);
                assert!(
                    draw_shape(canvas.dc, &shape, None),
                    "GDI draw failed for {shape:?}"
                );
                assert_ne!(GdiFlush(), 0, "GdiFlush after drawing {shape:?} failed");
                assert!(
                    canvas.changed_from(background),
                    "no pixels changed for {shape:?}"
                );
            }

            let mut source = DibCanvas::new(64, 64, [0, 0, 0, 255]);
            for y in 0..64 {
                for x in 0..64 {
                    let i = (y * 64 + x) * 4;
                    let v = if ((x / 4) + (y / 4)) % 2 == 0 {
                        16
                    } else {
                        224
                    };
                    source.bytes_mut()[i..i + 4].copy_from_slice(&[v, v, v, 255]);
                }
            }
            let destination = DibCanvas::new(64, 64, background);
            let blur = Shape::new(ShapeKind::Blur {
                start: Point::new(8.0, 8.0),
                end: Point::new(56.0, 56.0),
            });
            assert!(draw_shape(destination.dc, &blur, Some(source.dc)));
            assert_ne!(GdiFlush(), 0, "GdiFlush after blur failed");
            assert!(
                destination.changed_from(background),
                "blur did not change destination pixels"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn gdi_marker_alpha_blends_and_missing_blur_source_returns_false() {
        let background = [240, 240, 240, 255];
        unsafe {
            let canvas = DibCanvas::new(40, 40, background);
            let marker = Shape::new(ShapeKind::Highlight {
                start: Point::new(4.0, 4.0),
                end: Point::new(36.0, 36.0),
            });
            assert!(draw_shape(canvas.dc, &marker, None));
            assert_ne!(GdiFlush(), 0, "GdiFlush after marker draw failed");
            let blended = canvas.pixel(20, 20);
            assert_ne!(blended, background, "marker left destination unchanged");
            assert_ne!(blended, [0, 0, 255, 255], "marker was drawn fully opaque");
            assert_eq!(
                blended[3], 255,
                "opaque destination unexpectedly lost alpha"
            );

            let missing_source = DibCanvas::new(40, 40, background);
            let blur = Shape::new(ShapeKind::Blur {
                start: Point::new(4.0, 4.0),
                end: Point::new(36.0, 36.0),
            });
            assert!(!draw_shape(missing_source.dc, &blur, None));
        }
    }
