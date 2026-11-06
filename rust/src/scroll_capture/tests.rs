use super::*;

#[test]
fn settled_capture_returns_newest_sample_when_few_edge_rows_change() {
    struct Frames(std::collections::VecDeque<Image>);
    impl ScrollIo for Frames {
        fn capture(&mut self) -> Result<Image, String> {
            self.0
                .pop_front()
                .ok_or_else(|| "unexpected capture".into())
        }
        fn scroll(&mut self, _: Direction, _: i32) -> Result<(), String> {
            Ok(())
        }
        fn wait_ms(&mut self, _: u64) {}
        fn cancelled(&mut self) -> bool {
            false
        }
        fn mouse_moved(&mut self) -> bool {
            false
        }
        fn progress(&mut self, _: usize) {}
    }
    let first = Image::new(120, 200, [0, 0, 0, 255].repeat(120 * 200)).unwrap();
    let mut latest = first.clone();
    for y in 0..20 {
        latest.pixels[(y * 120 + 119) * 4] = 255;
    }
    let mut io = Frames([first, latest.clone()].into());
    let captured = capture_settled(&mut io, Direction::Down).unwrap();
    assert_eq!(captured.image, latest);
    assert!(io.0.is_empty());
}

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
    animate_at_end: bool,
    cancel: bool,
    mouse_move: bool,
    cancel_on_wait: Option<usize>,
    mouse_move_on_wait: Option<usize>,
    wait_calls: usize,
    capture_calls: usize,
    fail_capture_at: Option<usize>,
    scroll_calls: usize,
    progress: Vec<usize>,
    diagnostics: Vec<(Image, Image, String)>,
    diagnostic_success: bool,
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
            animate_at_end: false,
            cancel: false,
            mouse_move: false,
            cancel_on_wait: None,
            mouse_move_on_wait: None,
            wait_calls: 0,
            capture_calls: 0,
            fail_capture_at: None,
            scroll_calls: 0,
            progress: Vec::new(),
            diagnostics: Vec::new(),
            diagnostic_success: false,
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
            animate_at_end: false,
            cancel: false,
            mouse_move: false,
            cancel_on_wait: None,
            mouse_move_on_wait: None,
            wait_calls: 0,
            capture_calls: 0,
            fail_capture_at: None,
            scroll_calls: 0,
            progress: Vec::new(),
            diagnostics: Vec::new(),
            diagnostic_success: false,
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
    fn diagnostics(&mut self, before: &Image, after: &Image, details: &str) -> Option<String> {
        self.diagnostics
            .push((before.clone(), after.clone(), details.to_owned()));
        self.diagnostic_success.then(|| "diagnose".to_owned())
    }
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
                let mut v = if !self.horizontal && y < self.sticky_header {
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
                let viewport = if self.horizontal {
                    self.width
                } else {
                    self.height
                };
                let max_offset = self.page_len.saturating_sub(
                    viewport.saturating_sub(self.sticky_header + self.sticky_footer),
                );
                if self.animate_at_end && self.offset == max_offset {
                    // A full-width animated region replaces the whole viewport;
                    // independent per-pixel noise prevents accidental overlap matches.
                    let mut noise = (self.capture_calls as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                        ^ (y as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                        ^ (x as u64).wrapping_mul(0x94d0_49bb_1331_11eb);
                    noise ^= noise >> 30;
                    noise = noise.wrapping_mul(0xbf58_476d_1ce4_e5b9);
                    noise ^= noise >> 27;
                    noise = noise.wrapping_mul(0x94d0_49bb_1331_11eb);
                    noise ^= noise >> 31;
                    v = noise as u8;
                }
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

    let mut io = FakeScrollIo::vertical_with_sticky_bands(VIEW_H, CONTENT, HEADER, FOOTER);
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
    assert_eq!(
        (out.image.width, out.image.height),
        (WIDTH, HEADER + CONTENT + FOOTER)
    );
    assert_eq!(
        out.image.pixels, expected,
        "pixel mismatch at a seam or fixed band"
    );
    assert!(
        io.scroll_calls > 0 && io.scroll_calls <= 150,
        "scroll loop must remain bounded"
    );
    assert!(
        out.reason.contains("Ende des scrollbaren Bereichs"),
        "{}",
        out.reason
    );
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
    assert_eq!(
        io.capture_calls, 4,
        "the uncommitted second scroll must not be captured"
    );
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
    assert_eq!(
        io.capture_calls, 4,
        "the uncommitted second scroll must not be captured"
    );
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
fn conflicting_matchers_never_authorize_estimated_fallback() {
    let regular = MatchResult {
        dy: 30,
        top: 3,
        bottom: 4,
    };
    let exact = MatchResult {
        dy: 50,
        top: 0,
        bottom: 0,
    };
    let (result, strict) = reconcile_matches(Some(regular), Some(exact), false);
    assert!(result.is_none());
    assert!(
        strict,
        "conflict must latch strict mode before fallback selection"
    );
    let agreeing = MatchResult { dy: 50, ..regular };
    assert_eq!(
        reconcile_matches(Some(agreeing), Some(exact), false),
        (Some(agreeing), false)
    );
    assert_eq!(
        reconcile_matches(Some(agreeing), Some(exact), true),
        (Some(exact), true)
    );
}

#[test]
fn moving_irregular_content_only_at_outer_edge_stitches_exactly() {
    const WIDTH: usize = 120;
    const HEIGHT: usize = 220;
    const PAGE_LEN: usize = 700;

    struct EdgeOnlyPage {
        offset: usize,
        edge: usize,
        ambiguous_after: Option<usize>,
        scrollbar_after: Option<usize>,
    }
    impl EdgeOnlyPage {
        fn frame(&self) -> Image {
            let mut pixels = vec![255; WIDTH * HEIGHT * 4];
            for y in 0..HEIGHT {
                // A deterministic, irregular pattern moves with the page in
                // one outermost eight-pixel strip; the interior is uniform.
                let row = y + self.offset;
                let value = if self.ambiguous_after.is_some_and(|at| self.offset >= at) {
                    if y % 2 == 0 {
                        0
                    } else {
                        255
                    }
                } else if (row.wrapping_mul(73) ^ (row >> 2)) % 7 < 3 {
                    0
                } else {
                    255
                };
                for x in self.edge..self.edge + 8 {
                    let p = (y * WIDTH + x) * 4;
                    pixels[p..p + 3].fill(value);
                }
                if self.scrollbar_after.is_some_and(|at| self.offset >= at)
                    && (30..100).contains(&y)
                {
                    let x = if self.edge == 0 { WIDTH - 1 } else { 0 };
                    let p = (y * WIDTH + x) * 4;
                    pixels[p..p + 3].fill(127);
                }
            }
            Image::new(WIDTH, HEIGHT, pixels).unwrap()
        }
    }
    impl ScrollIo for EdgeOnlyPage {
        fn capture(&mut self) -> Result<Image, String> {
            Ok(self.frame())
        }
        fn scroll(&mut self, direction: Direction, notches: i32) -> Result<(), String> {
            if direction != Direction::Down {
                return Err("unexpected fake scroll direction".into());
            }
            let max_offset = PAGE_LEN - HEIGHT;
            self.offset = (self.offset as isize + notches.saturating_mul(50) as isize)
                .clamp(0, max_offset as isize) as usize;
            Ok(())
        }
        fn wait_ms(&mut self, _: u64) {}
        fn cancelled(&mut self) -> bool {
            false
        }
        fn mouse_moved(&mut self) -> bool {
            false
        }
        fn progress(&mut self, _: usize) {}
    }

    for edge in [0, WIDTH - 8] {
        let mut io = EdgeOnlyPage {
            offset: 0,
            edge,
            ambiguous_after: None,
            scrollbar_after: None,
        };
        let out = run(&mut io, Direction::Down).unwrap();
        assert_eq!(io.offset, PAGE_LEN - HEIGHT);
        assert_eq!(out.reason, "Ende des scrollbaren Bereichs erreicht.");
        assert_eq!(out.fallback_count, 0);
        assert_eq!((out.image.width, out.image.height), (WIDTH, PAGE_LEN));
        let mut expected = vec![255; WIDTH * PAGE_LEN * 4];
        for row in 0..PAGE_LEN {
            let value = if (row.wrapping_mul(73) ^ (row >> 2)) % 7 < 3 {
                0
            } else {
                255
            };
            for x in edge..edge + 8 {
                let p = (row * WIDTH + x) * 4;
                expected[p..p + 3].fill(value);
            }
        }
        assert_eq!(out.image.pixels, expected);

        let mut ambiguous = EdgeOnlyPage {
            offset: 0,
            edge,
            ambiguous_after: Some(100),
            scrollbar_after: None,
        };
        let partial = run(&mut ambiguous, Direction::Down).unwrap();
        assert!(
            partial.reason.contains("Aufnahme bis hierhin übernommen"),
            "{}",
            partial.reason
        );
        assert_eq!(partial.fallback_count, 0);
        assert_eq!(
            (partial.image.width, partial.image.height),
            (WIDTH, HEIGHT + 50)
        );
        assert_eq!(partial.image.pixels, expected[..WIDTH * (HEIGHT + 50) * 4]);

        let mut with_scrollbar = EdgeOnlyPage {
            offset: 0,
            edge,
            ambiguous_after: None,
            scrollbar_after: Some(100),
        };
        let partial = run(&mut with_scrollbar, Direction::Down).unwrap();
        assert!(
            partial.reason.contains("Aufnahme bis hierhin übernommen"),
            "{}",
            partial.reason
        );
        assert_eq!(partial.fallback_count, 0);
        assert_eq!(
            (partial.image.width, partial.image.height),
            (WIDTH, HEIGHT + 50)
        );
        assert_eq!(partial.image.pixels, expected[..WIDTH * (HEIGHT + 50) * 4]);
    }
}

#[test]
fn unconfirmed_zero_vote_after_progress_preserves_prefix_without_fallback() {
    const WIDTH: usize = 120;
    const HEIGHT: usize = 300;
    struct Page {
        offset: usize,
    }
    impl Page {
        fn frame(&self) -> Image {
            let mut pixels = vec![255; WIDTH * HEIGHT * 4];
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let value = if !(12..108).contains(&x) {
                        let row = y + self.offset;
                        if ((row * 73) ^ (row >> 2)) % 7 < 3 {
                            0
                        } else {
                            255
                        }
                    } else {
                        let row = y + self.offset.min(30);
                        let bin = (x - 12) / 12;
                        (25 + (row * 73 + bin * 41 + row * bin * 19 + row * row % 251) % 200
                            + if self.offset > 30 { 2 } else { 0 }) as u8
                    };
                    let p = (y * WIDTH + x) * 4;
                    pixels[p..p + 3].fill(value);
                }
            }
            Image::new(WIDTH, HEIGHT, pixels).unwrap()
        }
    }
    impl ScrollIo for Page {
        fn capture(&mut self) -> Result<Image, String> {
            Ok(self.frame())
        }
        fn scroll(&mut self, _: Direction, notches: i32) -> Result<(), String> {
            self.offset = (self.offset as i32 + notches * 30).max(0) as usize;
            Ok(())
        }
        fn wait_ms(&mut self, _: u64) {}
        fn cancelled(&mut self) -> bool {
            false
        }
        fn mouse_moved(&mut self) -> bool {
            false
        }
        fn progress(&mut self, _: usize) {}
    }
    let mut page = Page { offset: 0 };
    let first = page.frame();
    let second = Page { offset: 30 }.frame();
    let out = run(&mut page, Direction::Down).unwrap();
    assert!(
        out.reason.contains("nicht sicher nachweisbar"),
        "{}",
        out.reason
    );
    assert_eq!(out.fallback_count, 0);
    assert_eq!((out.image.width, out.image.height), (WIDTH, HEIGHT + 30));
    let mut expected = first.pixels;
    expected.extend_from_slice(&second.pixels[(HEIGHT - 30) * WIDTH * 4..]);
    assert_eq!(out.image.pixels, expected);
}

#[test]
fn animated_page_end_bounds_fallbacks_and_keeps_the_pixel_exact_prefix() {
    const VIEW_H: usize = 220;
    const COMMITTED_PREFIX: usize = 485;

    let mut io = FakeScrollIo::vertical(VIEW_H, 610);
    io.animate_at_end = true;
    let out = run(&mut io, Direction::Down).unwrap();

    assert!(io.diagnostics.is_empty());
    assert_eq!(
        out.fallback_count, 2,
        "only two consecutive guesses are allowed"
    );
    assert_eq!(io.scroll_calls, 10, "stop on the third unmatched end frame");
    assert!(
        out.reason.contains("Teilaufnahme bis hierhin übernommen"),
        "{}",
        out.reason
    );
    assert!(
        out.reason
            .contains("2× wurde die gemessene Scrollweite verwendet"),
        "{}",
        out.reason
    );
    assert!(
        out.reason.contains("Übergänge im Teilbild prüfen"),
        "{}",
        out.reason
    );

    // Seven confirmed movements have reached this viewport, but Stitcher has
    // only committed rows 0..485 before its current viewport tail. Fallback
    // frames may replace that tail and may contain gaps or duplicates; only
    // the committed prefix is guaranteed to remain pixel exact.
    let row_bytes = io.width * 4;
    let expected_prefix = io.capture_page_vertical(COMMITTED_PREFIX);
    assert!(out.image.height >= COMMITTED_PREFIX);
    let mismatch = out.image.pixels[..COMMITTED_PREFIX * row_bytes]
        .iter()
        .zip(&expected_prefix)
        .position(|(actual, expected)| actual != expected);
    assert!(
        mismatch.is_none(),
        "the reliably matched prefix must remain pixel exact; first mismatch row={:?}, progress={:?}",
        mismatch.map(|i| i / row_bytes),
        io.progress
    );
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

#[test]
fn horizontal_rotation_failures_preserve_the_last_accepted_capture() {
    for direction in [Direction::Right, Direction::Left] {
        for fail_at in [1, 2, 3, 4, 5] {
            let make_io = || match direction {
                Direction::Right => FakeScrollIo::horizontal(220, 260, 610),
                Direction::Left => FakeScrollIo::horizontal_left(220, 260, 610),
                Direction::Down => unreachable!(),
            };
            let mut expected_io = make_io();
            let first = expected_io.capture().unwrap();
            let mut io = make_io();
            let mut calls = 0;
            let out = run_with_rotation(&mut io, direction, &mut |image, rotation| {
                calls += 1;
                if calls == fail_at {
                    Err(RotationFailure {
                        image,
                        error: "simulierter Speichermangel",
                    })
                } else {
                    image.try_rotate(rotation)
                }
            })
            .unwrap();
            assert!(out.reason.contains("simulierter Speichermangel"));
            assert_eq!(out.pending_rotation, None);
            if fail_at <= 4 {
                assert_eq!(
                    out.image, first,
                    "direction={direction:?}, failure={fail_at}"
                );
            } else {
                assert!(out.image.width >= first.width && out.image.width <= 610);
                assert_eq!(out.image.height, first.height);
                let origin = if direction == Direction::Left {
                    610 - out.image.width
                } else {
                    0
                };
                for y in 0..out.image.height {
                    for x in 0..out.image.width {
                        let value = FakeScrollIo::pixel(origin + x, y);
                        let offset = (y * out.image.width + x) * 4;
                        assert_eq!(
                            &out.image.pixels[offset..offset + 4],
                            &[value, value, value, 255]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn persistent_rotation_failure_preserves_matcher_image_for_recovery() {
    for direction in [Direction::Right, Direction::Left] {
        for fail_at in [2, 3, 4, 5] {
            let mut io = match direction {
                Direction::Right => FakeScrollIo::horizontal(220, 260, 610),
                Direction::Left => FakeScrollIo::horizontal_left(220, 260, 610),
                Direction::Down => unreachable!(),
            };
            let mut calls = 0;
            let out = run_with_rotation(&mut io, direction, &mut |image, rotation| {
                calls += 1;
                if calls >= fail_at {
                    Err(RotationFailure {
                        image,
                        error: "simulierter Speichermangel",
                    })
                } else {
                    image.try_rotate(rotation)
                }
            })
            .unwrap();
            let required = out
                .pending_rotation
                .expect("failed final rotation must be recoverable");
            assert!(out.reason.contains("Wiederherstellung"));
            let image = out.image.try_rotate(required).unwrap();
            let origin = if direction == Direction::Left {
                610 - image.width
            } else {
                0
            };
            assert!(image.width >= 260 && image.width <= 610);
            assert_eq!(image.height, 220);
            for y in 0..image.height {
                for x in 0..image.width {
                    let value = FakeScrollIo::pixel(origin + x, y);
                    let offset = (y * image.width + x) * 4;
                    assert_eq!(
                        &image.pixels[offset..offset + 4],
                        &[value, value, value, 255]
                    );
                }
            }
        }
    }
}

#[test]
fn invalid_initial_rotation_image_is_not_delivered_as_a_capture() {
    let mut io = FakeScrollIo::horizontal(220, 260, 610);
    let result = run_with_rotation(&mut io, Direction::Right, &mut |_, _| {
        Err(RotationFailure {
            image: Image {
                width: 2,
                height: 2,
                pixels: vec![],
            },
            error: "Ungültige Bildgröße",
        })
    });
    assert_eq!(result.unwrap_err(), "Ungültige Bildgröße");
    assert_eq!(io.scroll_calls, 0);
}

#[test]
fn failed_final_rotation_retains_image_and_required_orientation() {
    for direction in [Direction::Right, Direction::Left] {
        let mut io = match direction {
            Direction::Right => FakeScrollIo::horizontal(220, 260, 610),
            Direction::Left => FakeScrollIo::horizontal_left(220, 260, 610),
            Direction::Down => unreachable!(),
        };
        let final_rotation = match direction {
            Direction::Right => Rotation::Counterclockwise,
            Direction::Left => Rotation::Clockwise,
            Direction::Down => unreachable!(),
        };
        let out = run_with_rotation(&mut io, direction, &mut |image, rotation| {
            if rotation == final_rotation {
                Err(RotationFailure {
                    image,
                    error: "simulierter Speichermangel",
                })
            } else {
                image.try_rotate(rotation)
            }
        })
        .unwrap();
        assert_eq!(out.pending_rotation, Some(final_rotation));
        assert!(out.reason.contains("Wiederherstellung"));
        let restored = out.image.try_rotate(final_rotation).unwrap();
        assert_eq!((restored.width, restored.height), (610, 220));
        assert_eq!(restored.pixels, io.capture_page_horizontal(610));
    }
}

#[test]
fn final_unmatched_first_step_saves_matching_frames_and_keeps_partial_image() {
    let mut io = FakeScrollIo::vertical(220, 270);
    io.animate_at_end = true;
    io.diagnostic_success = true;
    let initial = io.capture_page_vertical(220);
    let out = run(&mut io, Direction::Down).unwrap();
    assert_eq!(io.diagnostics.len(), 1);
    assert_eq!(io.diagnostics[0].0.pixels, initial);
    assert_ne!(io.diagnostics[0].1.pixels, initial);
    assert!(io.diagnostics[0].2.contains("Richtung: Down"));
    assert_eq!(out.image.pixels, initial);
    assert!(out.reason.contains("Diagnose gespeichert: diagnose"));
}

#[test]
fn diagnostic_write_failure_keeps_partial_image_without_claiming_a_path() {
    let mut io = FakeScrollIo::vertical(220, 270);
    io.animate_at_end = true;
    let initial = io.capture_page_vertical(220);
    let out = run(&mut io, Direction::Down).unwrap();
    assert_eq!(io.diagnostics.len(), 1);
    assert_eq!(out.image.pixels, initial);
    assert!(!out.reason.contains("Diagnose gespeichert"));
}
