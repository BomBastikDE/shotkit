//! Platform-independent scroll orchestration; the caller supplies screen capture,
//! scroll events, delays, cancellation and progress reporting.
use crate::scroll_match::{find_offset, Frame, MatchResult};
use crate::stitch::{Image, Stitcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Down,
    Right,
    Left,
}
#[derive(Debug)]
pub struct Outcome {
    pub image: Image,
    pub reason: String,
    pub fallback_count: usize,
}

pub trait ScrollIo {
    /// Frames are tightly packed BGRA, top down, in screen orientation.
    fn capture(&mut self) -> Result<Image, String>;
    /// Positive notches move in the selected direction; negative reverse them.
    fn scroll(&mut self, direction: Direction, notches: i32) -> Result<(), String>;
    fn wait_ms(&mut self, millis: u64);
    fn cancelled(&mut self) -> bool;
    fn mouse_moved(&mut self) -> bool;
    fn progress(&mut self, length: usize);
}
fn oriented(image: Image, direction: Direction) -> Image {
    match direction {
        Direction::Down => image,
        Direction::Right => image.clockwise(),
        Direction::Left => image.counterclockwise(),
    }
}
fn analyze(image: &Image) -> Result<Frame, String> {
    Frame::analyze(image.width, image.height, &image.pixels)
        .map_err(|e| format!("Bildanalyse fehlgeschlagen: {e:?}"))
}
fn interrupted(io: &mut impl ScrollIo) -> Option<&'static str> {
    if io.cancelled() {
        Some("Mit Esc beendet.")
    } else if io.mouse_moved() {
        Some("Durch Mausbewegung beendet.")
    } else {
        None
    }
}
struct Captured {
    image: Image,
    frame: Frame,
    error: Option<String>,
}
fn capture_settled(io: &mut impl ScrollIo, direction: Direction) -> Result<Captured, String> {
    let mut image = oriented(io.capture()?, direction);
    let mut frame = analyze(&image)?;
    for _ in 0..12 {
        if interrupted(io).is_some() {
            break;
        }
        io.wait_ms(90);
        // Keep the last frame already captured before the interrupt. In
        // particular, do not sample a possibly moving page after the wait.
        if interrupted(io).is_some() {
            break;
        }
        let next = match io.capture() {
            Ok(image) => oriented(image, direction),
            Err(error) => {
                return Ok(Captured {
                    image,
                    frame,
                    error: Some(error),
                })
            }
        };
        let next_frame = match analyze(&next) {
            Ok(frame) => frame,
            Err(error) => {
                return Ok(Captured {
                    image,
                    frame,
                    error: Some(error),
                })
            }
        };
        if frame.settled_with(&next_frame) {
            break;
        }
        image = next;
        frame = next_frame;
    }
    Ok(Captured {
        image,
        frame,
        error: None,
    })
}
/// Run up to 150 steps and return the usable portion even on cancellation or an
/// ambiguous match. The caller owns selection, UI, cursor restoration and diagnostics.
pub fn run(io: &mut impl ScrollIo, direction: Direction) -> Result<Outcome, String> {
    let initial = capture_settled(io, direction)?;
    let first = initial.image;
    let mut prev = initial.frame;
    let initial_error = initial.error;
    let h = first.height;
    if h < 200 {
        return Err("Der Bereich ist in Scrollrichtung kürzer als 200 Pixel.".into());
    }
    let mut stitch = Stitcher::new(first).map_err(str::to_owned)?;
    let (mut notches, mut expected, mut still) = (1i32, h / 3, 0usize);
    let (mut known, mut last_top, mut last_bottom, mut fallbacks) = (0f32, 0usize, 0usize, 0usize);
    let initial_stop = interrupted(io);
    let mut reason = initial_error
        .clone()
        .or_else(|| initial_stop.map(str::to_owned))
        .unwrap_or_else(|| "Maximale Anzahl Scroll-Schritte erreicht.".to_owned());
    if initial_error.is_none() && initial_stop.is_none() {
        for _step in 0..150 {
            if let Some(stop) = interrupted(io) {
                reason = stop.into();
                break;
            }
            if let Err(e) = io.scroll(direction, notches) {
                reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                break;
            }
            io.wait_ms(150);
            if let Some(stop) = interrupted(io) {
                reason = stop.into();
                break;
            }
            let captured = match capture_settled(io, direction) {
                Ok(frame) => frame,
                Err(e) => {
                    reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                    break;
                }
            };
            if let Some(e) = captured.error {
                reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                break;
            }
            let (mut cur, mut analyzed) = (captured.image, captured.frame);
            if let Some(stop) = interrupted(io) {
                reason = stop.into();
                break;
            }
            let matched = |a: &Frame, b: &Frame, expected| {
                if a.same(b) {
                    Some(MatchResult {
                        dy: 0,
                        top: 0,
                        bottom: 0,
                    })
                } else {
                    find_offset(a, b, expected)
                }
            };
            let mut m = matched(&prev, &analyzed, expected);
            if m.is_none() && notches > 1 {
                if let Err(e) = io.scroll(direction, -notches) {
                    reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                    break;
                }
                notches = (notches / 2).max(1);
                expected = (expected / 2).max(1);
                io.wait_ms(150);
                if let Some(stop) = interrupted(io) {
                    reason = stop.into();
                    break;
                }
                let captured = match capture_settled(io, direction) {
                    Ok(frame) => frame,
                    Err(e) => {
                        reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                        break;
                    }
                };
                if let Some(e) = captured.error {
                    reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                    break;
                }
                let (reverted, frame) = (captured.image, captured.frame);
                if let Some(stop) = interrupted(io) {
                    reason = stop.into();
                    break;
                }
                // The existing seam state still refers to the pre-scroll pixels. Only
                // proceed if the reverse motion really restored them.
                if !prev.settled_with(&frame) {
                    reason = "Scrollposition ließ sich nicht sicher wiederherstellen.".into();
                    break;
                }
                drop(reverted);
                prev = frame;
                continue;
            }
            if m.is_none() {
                io.wait_ms(700);
                if let Some(stop) = interrupted(io) {
                    reason = stop.into();
                    break;
                }
                let captured = match capture_settled(io, direction) {
                    Ok(frame) => frame,
                    Err(e) => {
                        reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                        break;
                    }
                };
                if let Some(e) = captured.error {
                    reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                    break;
                }
                (cur, analyzed) = (captured.image, captured.frame);
                if let Some(stop) = interrupted(io) {
                    reason = stop.into();
                    break;
                }
                m = matched(&prev, &analyzed, expected);
            }
            if m.is_none() && known > 0.0 {
                m = Some(MatchResult {
                    dy: (known * notches as f32).round() as usize,
                    top: last_top,
                    bottom: last_bottom,
                });
                fallbacks += 1;
            }
            let Some(m) = m else {
                reason="Bildinhalt ließ sich nicht eindeutig zuordnen – Aufnahme bis hierhin übernommen.".into();
                break;
            };
            if m.dy == 0 {
                still += 1;
                if still >= 2 {
                    reason = "Ende des scrollbaren Bereichs erreicht.".into();
                    break;
                }
                io.wait_ms(400);
                if let Some(stop) = interrupted(io) {
                    reason = stop.into();
                    break;
                }
                continue;
            }
            still = 0;
            let ppn = m.dy as f32 / notches as f32;
            if known == 0.0 || (ppn - known).abs() < 2.0 {
                known = ppn;
            }
            last_top = m.top;
            last_bottom = m.bottom;
            if let Err(e) = stitch.advance(cur, m.dy, m.top, m.bottom) {
                reason = format!("{e} – Aufnahme bis hierhin übernommen.");
                break;
            }
            prev = analyzed;
            let movable = h.saturating_sub(m.top + m.bottom);
            notches = ((movable as f32 * 0.4 / ppn.max(1.0)) as i32).clamp(1, 15);
            expected = (ppn * notches as f32) as usize;
            io.progress(stitch.length());
        }
    }
    if fallbacks > 0 {
        reason.push_str(&format!(
            "\n{fallbacks}× wurde die gemessene Scrollweite verwendet – Übergänge prüfen."
        ));
    }
    let combined = stitch.finish().map_err(str::to_owned)?;
    let image = match direction {
        Direction::Down => combined,
        Direction::Right => combined.counterclockwise(),
        Direction::Left => combined.clockwise(),
    };
    Ok(Outcome {
        image,
        reason,
        fallback_count: fallbacks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Virtual scrollable surface. In horizontal mode the screen viewport is
    /// moved across page columns; otherwise it moves across page rows.
    struct FakeScrollIo {
        width: usize,
        height: usize,
        page_len: usize,
        sticky_header: usize,
        sticky_footer: usize,
        offset: usize,
        horizontal: bool,
        cancel: bool,
        mouse_move: bool,
        cancel_on_wait: Option<usize>,
        mouse_move_on_wait: Option<usize>,
        wait_calls: usize,
        capture_calls: usize,
        fail_capture_at: Option<usize>,
        scroll_calls: usize,
        progress: Vec<usize>,
    }

    impl FakeScrollIo {
        fn vertical(height: usize, page_len: usize) -> Self {
            Self {
                width: 120,
                height,
                page_len,
                sticky_header: 0,
                sticky_footer: 0,
                offset: 0,
                horizontal: false,
                cancel: false,
                mouse_move: false,
                cancel_on_wait: None,
                mouse_move_on_wait: None,
                wait_calls: 0,
                capture_calls: 0,
                fail_capture_at: None,
                scroll_calls: 0,
                progress: Vec::new(),
            }
        }
        fn horizontal(height: usize, width: usize, page_len: usize) -> Self {
            Self {
                width,
                height,
                page_len,
                sticky_header: 0,
                sticky_footer: 0,
                offset: 0,
                horizontal: true,
                cancel: false,
                mouse_move: false,
                cancel_on_wait: None,
                mouse_move_on_wait: None,
                wait_calls: 0,
                capture_calls: 0,
                fail_capture_at: None,
                scroll_calls: 0,
                progress: Vec::new(),
            }
        }
        fn vertical_with_sticky_bands(
            height: usize,
            content_len: usize,
            header: usize,
            footer: usize,
        ) -> Self {
            assert!(header + footer + 24 < height);
            let mut io = Self::vertical(height, content_len);
            io.sticky_header = header;
            io.sticky_footer = footer;
            io
        }
        fn horizontal_left(height: usize, width: usize, page_len: usize) -> Self {
            let mut io = Self::horizontal(height, width, page_len);
            io.offset = page_len.saturating_sub(width);
            io
        }
        fn pixel(position: usize, cross: usize) -> u8 {
            ((position.wrapping_mul(73)
                + cross.wrapping_mul(41)
                + position.wrapping_mul(cross).wrapping_mul(19)
                + position.wrapping_mul(position) % 251)
                % 256) as u8
        }
        fn capture_page_vertical(&self, length: usize) -> Vec<u8> {
            let mut pixels = Vec::with_capacity(self.width * length * 4);
            for y in 0..length {
                for x in 0..self.width {
                    let v = Self::pixel(y, x);
                    pixels.extend_from_slice(&[v, v, v, 255]);
                }
            }
            pixels
        }
        fn capture_page_horizontal(&self, length: usize) -> Vec<u8> {
            let mut pixels = Vec::with_capacity(length * self.height * 4);
            for y in 0..self.height {
                for x in 0..length {
                    let v = Self::pixel(x, y);
                    pixels.extend_from_slice(&[v, v, v, 255]);
                }
            }
            pixels
        }
    }

    impl ScrollIo for FakeScrollIo {
        fn capture(&mut self) -> Result<Image, String> {
            self.capture_calls += 1;
            if self.fail_capture_at == Some(self.capture_calls) {
                return Err("fake capture failure".into());
            }
            let mut pixels = Vec::with_capacity(self.width * self.height * 4);
            for y in 0..self.height {
                for x in 0..self.width {
                    let (position, cross) = if self.horizontal {
                        (x + self.offset, y)
                    } else {
                        (y + self.offset, x)
                    };
                    let v = if !self.horizontal && y < self.sticky_header {
                        // A stationary, textured site header, visible in every frame.
                        Self::pixel(100_000 + y, x)
                    } else if !self.horizontal && y >= self.height - self.sticky_footer {
                        // A stationary, textured footer, also visible in every frame.
                        Self::pixel(200_000 + y, x)
                    } else if !self.horizontal && (self.sticky_header > 0 || self.sticky_footer > 0) {
                        let body_y = self.offset + y - self.sticky_header;
                        Self::pixel(300_000 + body_y, x)
                    } else {
                        Self::pixel(position, cross)
                    };
                    pixels.extend_from_slice(&[v, v, v, 255]);
                }
            }
            Image::new(self.width, self.height, pixels).map_err(str::to_owned)
        }
        fn scroll(&mut self, direction: Direction, notches: i32) -> Result<(), String> {
            if self.horizontal != (direction != Direction::Down) {
                return Err("unexpected fake scroll direction".into());
            }
            self.scroll_calls += 1;
            let viewport = if self.horizontal {
                self.width
            } else {
                self.height
            };
            let visible_content = viewport.saturating_sub(self.sticky_header + self.sticky_footer);
            let max = self.page_len.saturating_sub(visible_content);
            let signed_notches = match direction {
                Direction::Left => notches.saturating_neg(),
                _ => notches,
            };
            let delta = signed_notches.saturating_mul(50) as isize;
            self.offset = (self.offset as isize + delta).clamp(0, max as isize) as usize;
            Ok(())
        }
        fn wait_ms(&mut self, _: u64) {
            self.wait_calls += 1;
            if self.cancel_on_wait == Some(self.wait_calls) {
                self.cancel = true;
            }
            if self.mouse_move_on_wait == Some(self.wait_calls) {
                self.mouse_move = true;
            }
        }
        fn cancelled(&mut self) -> bool {
            self.cancel
        }
        fn mouse_moved(&mut self) -> bool {
            self.mouse_move
        }
        fn progress(&mut self, length: usize) {
            self.progress.push(length);
        }
    }

    #[test]
    fn down_stitches_a_moving_synthetic_page_to_its_full_length() {
        let mut io = FakeScrollIo::vertical(220, 610);
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(out.image.width, 120);
        assert_eq!(out.image.height, 610);
        assert_eq!(out.image.pixels, io.capture_page_vertical(610));
        assert!(
            out.reason.contains("Ende des scrollbaren Bereichs"),
            "{}",
            out.reason
        );
        assert!(io.scroll_calls >= 3);
        assert!(!io.progress.is_empty());
    }

    #[test]
    fn sticky_header_and_footer_are_not_duplicated_across_stitched_seams() {
        const VIEW_H: usize = 300;
        const WIDTH: usize = 120;
        const HEADER: usize = 36;
        const FOOTER: usize = 32;
        const CONTENT: usize = 1_100;

        let mut io = FakeScrollIo::vertical_with_sticky_bands(
            VIEW_H, CONTENT, HEADER, FOOTER,
        );
        let out = run(&mut io, Direction::Down).unwrap();

        // Exact full-image equality proves both seam pixels and that fixed bands
        // occur only once: initial header, every body row, final footer.
        let mut expected = Vec::with_capacity(WIDTH * (HEADER + CONTENT + FOOTER) * 4);
        for y in 0..HEADER {
            for x in 0..WIDTH {
                let p = FakeScrollIo::pixel(100_000 + y, x);
                expected.extend_from_slice(&[p, p, p, 255]);
            }
        }
        for y in 0..CONTENT {
            for x in 0..WIDTH {
                let p = FakeScrollIo::pixel(300_000 + y, x);
                expected.extend_from_slice(&[p, p, p, 255]);
            }
        }
        for y in VIEW_H - FOOTER..VIEW_H {
            for x in 0..WIDTH {
                let p = FakeScrollIo::pixel(200_000 + y, x);
                expected.extend_from_slice(&[p, p, p, 255]);
            }
        }
        assert_eq!((out.image.width, out.image.height), (WIDTH, HEADER + CONTENT + FOOTER));
        assert_eq!(out.image.pixels, expected, "pixel mismatch at a seam or fixed band");
        assert!(io.scroll_calls > 0 && io.scroll_calls <= 150, "scroll loop must remain bounded");
        assert!(out.reason.contains("Ende des scrollbaren Bereichs"), "{}", out.reason);
    }

    #[test]
    fn cancellation_returns_the_captured_prefix_without_scrolling() {
        let mut io = FakeScrollIo::vertical(220, 700);
        io.cancel = true;
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(out.reason, "Mit Esc beendet.");
        assert_eq!(out.image.height, 220);
        assert_eq!(io.scroll_calls, 0);
        assert_eq!(out.image.pixels, io.capture_page_vertical(220));
    }

    #[test]
    fn mouse_movement_before_first_scroll_keeps_initial_frame() {
        let mut io = FakeScrollIo::vertical(220, 700);
        io.mouse_move = true;
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(out.reason, "Durch Mausbewegung beendet.");
        assert_eq!(io.scroll_calls, 0);
        assert_eq!(out.image.pixels, io.capture_page_vertical(220));
    }

    #[test]
    fn cancellation_during_post_scroll_wait_does_not_capture_or_stitch_unsettled_frame() {
        let mut io = FakeScrollIo::vertical(220, 700);
        // Wait 1 settles the initial viewport; wait 2 follows the first scroll.
        io.cancel_on_wait = Some(2);
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(out.reason, "Mit Esc beendet.");
        assert_eq!(io.scroll_calls, 1);
        assert_eq!(io.capture_calls, 2);
        assert_eq!(out.image.height, 220);
        assert_eq!(out.image.pixels, io.capture_page_vertical(220));
    }

    #[test]
    fn cancellation_after_a_successful_stitch_keeps_the_pixel_exact_prefix() {
        let mut io = FakeScrollIo::vertical(220, 700);
        // Waits 1-3 settle the initial and first scrolled viewports. The first
        // 50px overlap is stitched before wait 4, after the next scroll event.
        io.cancel_on_wait = Some(4);
        let out = run(&mut io, Direction::Down).unwrap();

        assert_eq!(out.reason, "Mit Esc beendet.");
        assert_eq!(io.scroll_calls, 2);
        assert_eq!(io.capture_calls, 4, "the uncommitted second scroll must not be captured");
        assert_eq!((out.image.width, out.image.height), (120, 270));
        assert_eq!(out.image.pixels, io.capture_page_vertical(270));
    }

    #[test]
    fn mouse_movement_after_a_successful_stitch_keeps_the_pixel_exact_prefix() {
        let mut io = FakeScrollIo::vertical(220, 700);
        io.mouse_move_on_wait = Some(4);
        let out = run(&mut io, Direction::Down).unwrap();

        assert_eq!(out.reason, "Durch Mausbewegung beendet.");
        assert_eq!(io.scroll_calls, 2);
        assert_eq!(io.capture_calls, 4, "the uncommitted second scroll must not be captured");
        assert_eq!((out.image.width, out.image.height), (120, 270));
        assert_eq!(out.image.pixels, io.capture_page_vertical(270));
    }

    #[test]
    fn capture_failure_after_initial_frame_returns_captured_prefix() {
        let mut io = FakeScrollIo::vertical(220, 700);
        // Two captures establish the initial settled frame; the first post-scroll capture fails.
        io.fail_capture_at = Some(3);
        let out = run(&mut io, Direction::Down).unwrap();
        assert!(
            out.reason.contains("fake capture failure"),
            "{}",
            out.reason
        );
        assert!(out.reason.contains("Aufnahme bis hierhin übernommen"));
        assert_eq!(out.image.height, 220);
        assert_eq!(out.image.pixels, io.capture_page_vertical(220));
    }

    #[test]
    fn static_page_is_detected_as_end_after_two_attempts() {
        let mut io = FakeScrollIo::vertical(220, 220);
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(out.reason, "Ende des scrollbaren Bereichs erreicht.");
        assert_eq!(out.image.height, 220);
        assert_eq!(io.scroll_calls, 2);
    }

    #[test]
    fn rightward_capture_rotates_back_to_output_orientation() {
        let mut io = FakeScrollIo::horizontal(220, 260, 610);
        let out = run(&mut io, Direction::Right).unwrap();
        assert_eq!((out.image.width, out.image.height), (610, 220));
        assert_eq!(out.image.pixels, io.capture_page_horizontal(610));
        assert!(
            out.reason.contains("Ende des scrollbaren Bereichs"),
            "{}",
            out.reason
        );
        assert!(io.scroll_calls >= 3);
    }

    #[test]
    fn leftward_capture_starts_at_right_edge_and_stitches_back_in_page_order() {
        let mut io = FakeScrollIo::horizontal_left(220, 260, 610);
        assert_eq!(
            io.offset, 350,
            "leftward capture must start at the page's right edge"
        );
        let out = run(&mut io, Direction::Left).unwrap();
        assert_eq!((out.image.width, out.image.height), (610, 220));
        assert_eq!(
            out.image.pixels,
            io.capture_page_horizontal(610),
            "all pixels and seams must return in left-to-right page order"
        );
        assert_eq!(io.offset, 0, "capture should reach the page's left edge");
        assert!(
            out.reason.contains("Ende des scrollbaren Bereichs"),
            "{}",
            out.reason
        );
        assert!(io.scroll_calls >= 3);
    }
}
