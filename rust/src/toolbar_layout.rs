//! Platform-independent geometry of the native editor toolbar.
const MARGIN: i32 = 4;
const GAP: i32 = 3;
const BUTTON_SIZE: i32 = 36;

pub type ButtonRect = (i32, i32, i32, i32);
pub type ToolbarLayout = (i32, i32, Vec<ButtonRect>);

pub fn layout(width: i32, action_count: usize) -> ToolbarLayout {
    let width = width.max(1);
    let available = (width - 2 * MARGIN).max(1);
    let columns = ((available + GAP) / (BUTTON_SIZE + GAP)).max(1) as usize;
    let columns = columns.min(action_count);
    let rows = action_count.div_ceil(columns);
    let button_width = ((available - GAP * (columns as i32 - 1)) / columns as i32).max(1);
    let height = MARGIN * 2 + rows as i32 * BUTTON_SIZE + (rows as i32 - 1) * GAP;
    let positions = (0..action_count)
        .map(|index| {
            let col = index % columns;
            let row = index / columns;
            (
                MARGIN + col as i32 * (button_width + GAP),
                MARGIN + row as i32 * (BUTTON_SIZE + GAP),
                button_width,
                BUTTON_SIZE,
            )
        })
        .collect();
    (columns as i32, height, positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_toolbar_places_every_action_in_one_row() {
        let (columns, height, positions) = layout(1000, 24);
        assert_eq!((columns, height, positions.len()), (24, 44, 24));
        assert_eq!(positions[0].0, 4);
        assert_eq!(positions[23].1, 4);
    }

    #[test]
    fn narrow_toolbar_wraps_without_losing_or_overlapping_actions() {
        let (columns, height, positions) = layout(120, 24);
        assert_eq!(columns, 2);
        assert_eq!(height, 473);
        assert_eq!(positions.len(), 24);
        assert_eq!(positions[0], (4, 4, 54, 36));
        assert_eq!(positions[1], (61, 4, 54, 36));
        assert_eq!(positions[2], (4, 43, 54, 36));
        assert_eq!(positions[23].1 + positions[23].3, height - MARGIN);
    }
}
