//! Platform-independent scroll orchestration; the caller supplies screen capture,
//! scroll events, delays, cancellation and progress reporting.
use crate::scroll_edge_match::find_edge_offset;
use crate::scroll_match::{find_offset, Frame, MatchResult};
use crate::stitch::{Image, Rotation, RotationFailure, Stitcher};

const MAX_CONSECUTIVE_FALLBACKS: usize = 2;

// Screen pixels prove motion or a lack of motion, never the document boundary.
// Keep this warning on every usable result, including cancellation/error paths.
const UNCONFIRMED_END: &str = "Möglicherweise unvollständig: Das Seitenende wurde nicht bestätigt. Die bisherige Aufnahme bleibt erhalten. Bitte das Ende und die Übergänge im Bild prüfen.";
fn unconfirmed_reason(reason: &str) -> String {
    format!("{UNCONFIRMED_END}\n\n{reason}")
}

fn reconcile_matches(
    regular: Option<MatchResult>,
    exact: Option<MatchResult>,
    exact_mode: bool,
) -> (Option<MatchResult>, bool) {
    if let Some(exact) = exact {
        if let Some(regular) = regular.filter(|result| result.dy > 0) {
            if regular.dy != exact.dy {
                return (None, true);
            }
            if !exact_mode {
                return (Some(regular), false);
            }
        }
        return (Some(exact), true);
    }
    if exact_mode {
        (regular.filter(|result| result.dy == 0), false)
    } else {
        (regular, false)
    }
}

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
    /// A horizontal result still in matcher orientation; it must enter recovery
    /// and must not be automatically copied or saved before this rotation.
    pub pending_rotation: Option<Rotation>,
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
fn oriented(
    image: Image,
    direction: Direction,
    rotate: &mut impl FnMut(Image, Rotation) -> Result<Image, RotationFailure>,
) -> Result<Image, RotationFailure> {
    match direction {
        Direction::Down => Ok(image),
        Direction::Right => rotate(image, Rotation::Clockwise),
        Direction::Left => rotate(image, Rotation::Counterclockwise),
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
#[derive(Debug)]
enum CaptureError {
    Capture(String),
    Rotation(RotationFailure),
}

impl CaptureError {
    fn message(&self) -> String {
        match self {
            Self::Capture(error) => error.clone(),
            Self::Rotation(error) => error.error.to_owned(),
        }
    }
}

fn capture_settled_with_rotation(
    io: &mut impl ScrollIo,
    direction: Direction,
    rotate: &mut impl FnMut(Image, Rotation) -> Result<Image, RotationFailure>,
) -> Result<Captured, CaptureError> {
    let image = io.capture().map_err(CaptureError::Capture)?;
    let mut image = oriented(image, direction, rotate).map_err(CaptureError::Rotation)?;
    let mut frame = analyze(&image).map_err(CaptureError::Capture)?;
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
            Ok(next) => match oriented(next, direction, rotate) {
                Ok(next) => next,
                Err(error) => {
                    return Ok(Captured {
                        image,
                        frame,
                        error: Some(error.error.to_owned()),
                    });
                }
            },
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

#[cfg(test)]
fn capture_settled(io: &mut impl ScrollIo, direction: Direction) -> Result<Captured, CaptureError> {
    capture_settled_with_rotation(io, direction, &mut |image, rotation| {
        image.try_rotate(rotation)
    })
}
/// Run up to 150 steps and return the usable portion even on cancellation or an
/// ambiguous match. The caller owns selection, UI, cursor restoration and diagnostics.
pub fn run(io: &mut impl ScrollIo, direction: Direction) -> Result<Outcome, String> {
    run_with_rotation(io, direction, &mut |image, rotation| {
        image.try_rotate(rotation)
    })
}

fn run_with_rotation(
    io: &mut impl ScrollIo,
    direction: Direction,
    rotate: &mut impl FnMut(Image, Rotation) -> Result<Image, RotationFailure>,
) -> Result<Outcome, String> {
    let initial = match capture_settled_with_rotation(io, direction, rotate) {
        Ok(initial) => initial,
        Err(CaptureError::Capture(error)) => return Err(error),
        Err(CaptureError::Rotation(error)) => {
            // Only a valid captured screen frame can be delivered as the
            // fallback image. A malformed public Image must remain an error.
            if error.image.validate().is_err() {
                return Err(error.error.to_owned());
            }
            return Ok(Outcome {
                image: error.image,
                reason: unconfirmed_reason(&format!("Scrollaufnahme konnte nicht gestartet werden: {}. Die erste Aufnahme bleibt erhalten.", error.error)),
                fallback_count: 0,
                pending_rotation: None,
            });
        }
    };
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
    let mut exact_edge_mode = false;
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
            let captured = match capture_settled_with_rotation(io, direction, rotate) {
                Ok(frame) => frame,
                Err(e) => {
                    reason = format!("{} – Aufnahme bis hierhin übernommen.", e.message());
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
            let match_frames = |old: &Frame, new: &Frame, current: &Image, exact_mode: bool| {
                let regular = matched(old, new, expected);
                if old.same(new) {
                    return (regular, false);
                }
                let exact = find_edge_offset(
                    restored_image.as_ref().unwrap_or_else(|| stitch.frame()),
                    current,
                );
                reconcile_matches(regular, exact, exact_mode)
            };
            let (mut m, activate_exact) = match_frames(&prev, &analyzed, &cur, exact_edge_mode);
            exact_edge_mode |= activate_exact;
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
                let captured = match capture_settled_with_rotation(io, direction, rotate) {
                    Ok(frame) => frame,
                    Err(e) => {
                        reason = format!("{} – Aufnahme bis hierhin übernommen.", e.message());
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
                let captured = match capture_settled_with_rotation(io, direction, rotate) {
                    Ok(frame) => frame,
                    Err(e) => {
                        reason = format!("{} – Aufnahme bis hierhin übernommen.", e.message());
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
                let (new_match, activate_exact) =
                    match_frames(&prev, &analyzed, &cur, exact_edge_mode);
                m = new_match;
                exact_edge_mode |= activate_exact;
            }
            let used_fallback = m.is_none() && known > 0.0 && !exact_edge_mode;
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
                if !prev.same(&analyzed) {
                    reason = "Scrollstillstand ist wegen veränderter Bildinhalte nicht sicher nachweisbar – Teilaufnahme bis hierhin übernommen.".into();
                    break;
                }
                still += 1;
                if still >= 2 {
                    reason = "Keine weitere Scrollbewegung erkennbar.".into();
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
    let final_rotation = match direction {
        Direction::Down => None,
        Direction::Right => Some(Rotation::Counterclockwise),
        Direction::Left => Some(Rotation::Clockwise),
    };
    let (image, pending_rotation) = match final_rotation {
        None => (combined, None),
        Some(rotation) => match rotate(combined, rotation) {
            Ok(image) => (image, None),
            Err(error) => {
                reason.push_str(&format!("\nDie Aufnahme konnte nicht zurückgedreht werden: {}. Sie bleibt zur Wiederherstellung im Speicher.", error.error));
                (error.image, Some(rotation))
            }
        },
    };
    Ok(Outcome {
        image,
        reason: unconfirmed_reason(&reason),
        fallback_count: fallbacks,
        pending_rotation,
    })
}

#[cfg(test)]
mod tests;
