use super::*;

/// Draw the preview with higher-quality filtering when reducing the image.
// SAFETY CONTRACT:
// Voraussetzung: dc und src sind gültige GDI-DCs, src enthält das Bild, und
// Ziel-/Quellmaße beschreiben gültige Rechtecke während dieses Aufrufs.
// Warum erfüllt: Der Paint-Handler besitzt beide DCs und berechnet die Maße
// vor dem Aufruf; geänderter GDI-Zustand wird vor der Rückkehr zurückgesetzt.
// Nachrichten: Die GDI-Aufrufe senden keine expliziten Fensternachrichten.
pub(super) unsafe fn blit_preview(
    dc: H,
    src: H,
    target: (i32, i32, i32, i32),
    source: (i32, i32),
    high_quality: bool,
) -> bool {
    let (x, y, width, height) = target;
    let smooth = high_quality && view::smooth_preview(source, (width, height));
    let mut previous_mode = if smooth { SetStretchBltMode(dc, 4) } else { 0 }; // HALFTONE
    let mut previous_origin = Point { x: 0, y: 0 };
    let origin_set = previous_mode != 0 && SetBrushOrgEx(dc, 0, 0, &mut previous_origin) != 0;
    if previous_mode != 0 && !origin_set {
        SetStretchBltMode(dc, previous_mode);
        previous_mode = 0;
    }
    let ok = StretchBlt(dc, x, y, width, height, src, 0, 0, source.0, source.1, COPY) != 0;
    if origin_set {
        SetBrushOrgEx(dc, previous_origin.x, previous_origin.y, null_mut());
    }
    if previous_mode != 0 {
        SetStretchBltMode(dc, previous_mode);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateCompatibleBitmap(dc: H, width: i32, height: i32) -> H;
        fn SetPixel(dc: H, x: i32, y: i32, color: u32) -> u32;
        fn GetPixel(dc: H, x: i32, y: i32) -> u32;
    }

    struct BitmapDc {
        dc: H,
        bitmap: H,
        old: H,
    }
    impl Drop for BitmapDc {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }
    // SAFETY CONTRACT:
    // Voraussetzung: screen ist ein gültiger Bildschirm-DC und Maße sind positiv.
    // Warum erfüllt: Der Test prüft GetDC und verwendet feste positive Maße;
    // BitmapDc stellt Auswahl und Ressourcen wieder her.
    // Nachrichten: GDI-Funktionen versenden keine expliziten Fensternachrichten.
    unsafe fn bitmap_dc(screen: H, width: i32, height: i32) -> BitmapDc {
        let dc = CreateCompatibleDC(screen);
        assert!(!dc.is_null());
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        assert!(!bitmap.is_null());
        let old = SelectObject(dc, bitmap);
        assert!(!old.is_null() && old as isize != -1);
        BitmapDc { dc, bitmap, old }
    }

    #[test]
    fn reduced_preview_blends_detail_and_native_size_preserves_pixels() {
        unsafe {
            let screen = GetDC(null_mut());
            assert!(!screen.is_null());
            let src = bitmap_dc(screen, 16, 16);
            let reduced = bitmap_dc(screen, 4, 4);
            let native = bitmap_dc(screen, 16, 16);
            ReleaseDC(null_mut(), screen);
            for y in 0..16 {
                for x in 0..16 {
                    assert_ne!(
                        SetPixel(src.dc, x, y, if (x + y) % 2 == 0 { 0 } else { 0x00ff_ffff }),
                        u32::MAX
                    );
                }
            }
            assert!(blit_preview(
                reduced.dc,
                src.dc,
                (0, 0, 4, 4),
                (16, 16),
                true
            ));
            let pixel = GetPixel(reduced.dc, 1, 1);
            let red = pixel & 255;
            assert!(
                red > 0 && red < 255,
                "reduced checkerboard must blend, got {pixel:08x}"
            );
            assert!(blit_preview(
                reduced.dc,
                src.dc,
                (0, 0, 4, 4),
                (16, 16),
                false
            ));
            let fast_red = GetPixel(reduced.dc, 1, 1) & 255;
            assert!(
                fast_red == 0 || fast_red == 255,
                "live drawing should use the fast path"
            );
            assert!(blit_preview(
                native.dc,
                src.dc,
                (0, 0, 16, 16),
                (16, 16),
                true
            ));
            for y in 0..16 {
                for x in 0..16 {
                    assert_eq!(GetPixel(native.dc, x, y), GetPixel(src.dc, x, y));
                }
            }
        }
    }
}
