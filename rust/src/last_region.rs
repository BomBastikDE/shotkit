//! Geometry helpers for validating a remembered screen capture against the
//! monitors currently attached to the desktop.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn from_origin_size(x: i32, y: i32, width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(Self {
            left: x,
            top: y,
            right: x.checked_add(width)?,
            bottom: y.checked_add(height)?,
        })
    }

    fn valid(self) -> bool {
        self.right > self.left && self.bottom > self.top
    }
}

/// True only when the complete half-open selection is covered by the union
/// of current monitor rectangles. Desktop gaps and detached-monitor remnants
/// therefore fail closed.
pub fn fully_covered_by_monitors(selection: Rect, monitors: &[Rect]) -> bool {
    if !selection.valid() || monitors.is_empty() {
        return false;
    }

    let mut y_edges = vec![selection.top, selection.bottom];
    for monitor in monitors.iter().copied().filter(|r| r.valid()) {
        let top = selection.top.max(monitor.top);
        let bottom = selection.bottom.min(monitor.bottom);
        if top < bottom {
            y_edges.push(top);
            y_edges.push(bottom);
        }
    }
    y_edges.sort_unstable();
    y_edges.dedup();

    for band in y_edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        if top == bottom {
            continue;
        }
        let mut spans: Vec<(i32, i32)> = monitors
            .iter()
            .copied()
            .filter(|r| r.valid() && r.top <= top && r.bottom >= bottom)
            .map(|r| (r.left.max(selection.left), r.right.min(selection.right)))
            .filter(|(left, right)| left < right)
            .collect();
        spans.sort_unstable();

        let mut covered_until = selection.left;
        for (left, right) in spans {
            if left > covered_until {
                break;
            }
            covered_until = covered_until.max(right);
            if covered_until >= selection.right {
                break;
            }
        }
        if covered_until < selection.right {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{fully_covered_by_monitors, Rect};

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn accepts_negative_origin_and_adjacent_mixed_size_monitors() {
        let monitors = [rect(-1920, 0, 0, 1080), rect(0, -200, 2560, 1240)];
        assert!(fully_covered_by_monitors(rect(-100, 50, 1200, 800), &monitors));
    }

    #[test]
    fn rejects_selection_outside_current_monitor_bounds() {
        let monitors = [rect(0, 0, 1920, 1080)];
        assert!(!fully_covered_by_monitors(rect(1800, 900, 2000, 1100), &monitors));
        assert!(!fully_covered_by_monitors(rect(-1, 0, 10, 10), &monitors));
    }

    #[test]
    fn rejects_gaps_between_monitors_even_inside_virtual_bounds() {
        let monitors = [rect(-1000, 0, 0, 800), rect(100, 0, 1100, 800)];
        assert!(!fully_covered_by_monitors(rect(-10, 100, 110, 200), &monitors));
    }

    #[test]
    fn accepts_union_coverage_without_requiring_one_monitor_to_contain_area() {
        let monitors = [rect(0, 0, 100, 100), rect(100, 0, 200, 100)];
        assert!(fully_covered_by_monitors(rect(20, 20, 180, 80), &monitors));
    }

    #[test]
    fn rejects_invalid_selection_and_empty_monitor_set() {
        assert!(!fully_covered_by_monitors(rect(10, 10, 10, 20), &[rect(0, 0, 30, 30)]));
        assert!(!fully_covered_by_monitors(rect(1, 1, 2, 2), &[]));
        assert_eq!(Rect::from_origin_size(1, 1, 0, 2), None);
    }
}
