//! Platform-independent scroll orchestration; the caller supplies screen capture,
//! scroll events, delays, cancellation and progress reporting.
use crate::scroll_match::{find_offset, Frame, MatchResult};
use crate::stitch::{Image, Stitcher};

const MAX_CONSECUTIVE_FALLBACKS: usize = 2;

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
    /// Called only after the final matching attempt fails. Images are in
    /// matcher orientation; the adapter may persist them without affecting capture.
    fn diagnostics(&mut self, _before: &Image, _after: &Image, _details: &str) -> Option<String> {
        None
    }
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
            // The newest sample can still differ in up to 10% of rows.
            // Returning the previous sample would silently discard those pixels.
            image = next;
            frame = next_frame;
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
    let mut consecutive_fallbacks = 0usize;
    let mut restored_image: Option<Image> = None;
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
                restored_image = Some(reverted);
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
            let used_fallback = m.is_none() && known > 0.0;
            if used_fallback && consecutive_fallbacks >= MAX_CONSECUTIVE_FALLBACKS {
                reason = format!(
                    "{} aufeinanderfolgende Bildabgleiche fehlgeschlagen – Teilaufnahme bis hierhin übernommen.",
                    consecutive_fallbacks + 1
                );
                break;
            }
            if used_fallback {
                m = Some(MatchResult {
                    dy: (known * notches as f32).round() as usize,
                    top: last_top,
                    bottom: last_bottom,
                });
                fallbacks += 1;
                consecutive_fallbacks += 1;
            } else if m.is_some() {
                consecutive_fallbacks = 0;
            }
            let Some(m) = m else {
                reason="Bildinhalt ließ sich nicht eindeutig zuordnen – Aufnahme bis hierhin übernommen.".into();
                let details = format!("Richtung: {direction:?}\nErwartete Verschiebung: {expected} px\nScrollrasten: {notches}\nGemessene Pixel pro Raste: {known}\nLetzte feste Ränder: oben {last_top} px, unten {last_bottom} px\nAufeinanderfolgende Ersatzabgleiche: {consecutive_fallbacks}\nAbgleich: kein eindeutiges Ergebnis nach erneutem Versuch\nBilder: Matcher-Ausrichtung (bei horizontaler Aufnahme um 90 Grad gedreht)\n");
                if let Some(path) = io.diagnostics(
                    restored_image.as_ref().unwrap_or_else(|| stitch.frame()),
                    &cur,
                    &details,
                ) {
                    reason.push_str(&format!("\nDiagnose gespeichert: {path}"));
                }
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
            restored_image = None;
            prev = analyzed;
            let movable = h.saturating_sub(m.top + m.bottom);
            notches = ((movable as f32 * 0.4 / ppn.max(1.0)) as i32).clamp(1, 15);
            expected = (ppn * notches as f32) as usize;
            io.progress(stitch.length());
        }
    }
    if fallbacks > 0 {
        reason.push_str(&format!(
            "\n{fallbacks}× wurde die gemessene Scrollweite verwendet – Übergänge im Teilbild prüfen."
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
mod tests;
