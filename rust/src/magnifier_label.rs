//! Format the selection magnifier's screen coordinates and sampled GDI color.

/// `color` is a Win32 COLORREF (0x00BBGGRR), or CLR_INVALID on failure.
pub fn format_label(x: i32, y: i32, color: u32) -> String {
    if color == 0xffff_ffff {
        format!("{x}, {y}")
    } else {
        format!(
            "{x}, {y}  #{:02X}{:02X}{:02X}",
            color & 0xff,
            (color >> 8) & 0xff,
            (color >> 16) & 0xff
        )
    }
}

#[cfg(test)]
mod tests {
    use super::format_label;

    #[test]
    fn displays_screen_coordinates_and_colorref_in_rgb_order() {
        assert_eq!(format_label(-1920, 47, 0x00e7_a412), "-1920, 47  #12A4E7");
        assert_eq!(format_label(0, -1, 0), "0, -1  #000000");
        assert_eq!(format_label(17, 9, 0x00ff_ffff), "17, 9  #FFFFFF");
    }

    #[test]
    fn unavailable_pixel_displays_coordinates_without_a_color() {
        assert_eq!(format_label(-1, -2, 0xffff_ffff), "-1, -2");
    }
}
