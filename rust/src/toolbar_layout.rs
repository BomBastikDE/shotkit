//! Platform-independent, single-row toolbar geometry and staged overflow policy.
//! Sizes are logical pixels; native controls and popup anchors use the same DPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Tool,
    Palette,
    SizeHistory,
    View,
    Fixed,
}
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: usize,
    pub group: Group,
}
pub type ButtonRect = (i32, i32, i32, i32);
#[derive(Debug)]
pub struct Layout {
    pub stage: u8,
    pub height: i32,
    pub buttons: Vec<(usize, ButtonRect)>,
    pub overflow: Vec<usize>,
    pub more: Option<ButtonRect>,
}

pub fn scale(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi.clamp(96, 768)) + 48) / 96) as i32
}

/// Fixed actions reserve the right edge before optional groups are considered.
/// At unusually tiny sizes, controls compress horizontally to retain the four
/// fixed actions and overflow button. No minimum editor window is imposed.
pub fn layout(width: i32, dpi: u32, items: &[Item]) -> Layout {
    let width = width.max(1);
    let fixed = items.iter().filter(|i| i.group == Group::Fixed).count();
    let slots = fixed as i32 + 1;
    let margin = scale(4, dpi).min(width / (slots * 2 + 2));
    let gap = scale(3, dpi).min((width - 2 * margin) / (slots * 2));
    let size = scale(32, dpi).min(((width - 2 * margin - (slots - 1) * gap) / slots).max(1));
    let top = scale(4, dpi);
    let height = scale(36, dpi);
    let fits = |count: usize| 2 * margin + count as i32 * (size + gap) - gap <= width;
    let hidden = |group, stage| match group {
        Group::View => stage >= 1,
        Group::SizeHistory => stage >= 2,
        Group::Palette => stage >= 3,
        _ => false,
    };
    let mut stage = 0;
    while stage < 3 {
        let count = items.iter().filter(|i| !hidden(i.group, stage)).count();
        if fits(count + usize::from(stage > 0)) {
            break;
        }
        stage += 1;
    }
    let mut visible: Vec<_> = items
        .iter()
        .filter(|i| !hidden(i.group, stage))
        .map(|i| i.id)
        .collect();
    // The C# left ToolStrip also overflows its remaining tools when necessary.
    while !fits(visible.len() + usize::from(stage > 0)) {
        let Some(index) = items
            .iter()
            .rev()
            .find(|i| i.group != Group::Fixed && visible.contains(&i.id))
        else {
            break;
        };
        visible.retain(|id| *id != index.id);
    }
    let overflow: Vec<_> = items
        .iter()
        .filter(|i| !visible.contains(&i.id))
        .map(|i| i.id)
        .collect();
    let mut buttons = Vec::new();
    let mut left = margin;
    for item in items
        .iter()
        .filter(|i| i.group != Group::Fixed && visible.contains(&i.id))
    {
        buttons.push((item.id, (left, top, size, height)));
        left += size + gap;
    }
    let more = (!overflow.is_empty()).then_some((left, top, size, height));
    let mut right = width - margin - fixed as i32 * (size + gap) + gap;
    for item in items.iter().filter(|i| i.group == Group::Fixed) {
        buttons.push((item.id, (right, top, size, height)));
        right += size + gap;
    }
    Layout {
        stage,
        height: height + 2 * top,
        buttons,
        overflow,
        more,
    }
}

pub fn minimum_width(dpi: u32, fixed: usize) -> i32 {
    2 * scale(4, dpi) + (fixed as i32 + 1) * (scale(32, dpi) + scale(3, dpi)) - scale(3, dpi)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn items() -> Vec<Item> {
        let groups = [
            Group::Tool,
            Group::Tool,
            Group::Palette,
            Group::SizeHistory,
            Group::View,
            Group::View,
            Group::Fixed,
            Group::Fixed,
            Group::Fixed,
            Group::Fixed,
        ];
        groups
            .into_iter()
            .enumerate()
            .map(|(id, group)| Item { id, group })
            .collect()
    }
    #[test]
    fn stages_move_groups_in_reference_order_and_restore_on_growth() {
        let items = items();
        for (width, stage, hidden) in [
            (355, 0, vec![]),
            (354, 1, vec![4, 5]),
            (319, 2, vec![3, 4, 5]),
            (284, 3, vec![2, 3, 4, 5]),
        ] {
            let plan = layout(width, 96, &items);
            assert_eq!(plan.stage, stage, "width {width}");
            assert_eq!(plan.overflow, hidden, "width {width}");
            assert_eq!(plan.height, 44);
        }
        assert!(layout(1000, 96, &items).overflow.is_empty());
    }
    #[test]
    fn every_action_is_visible_or_reachable_once_and_fixed_actions_stay_right() {
        let items = items();
        for dpi in [96, 120, 144, 192, 288] {
            for width in [
                minimum_width(dpi, 4),
                scale(250, dpi),
                scale(350, dpi),
                scale(1000, dpi),
            ] {
                let plan = layout(width, dpi, &items);
                let mut ids: Vec<_> = plan
                    .buttons
                    .iter()
                    .map(|p| p.0)
                    .chain(plan.overflow.iter().copied())
                    .collect();
                ids.sort_unstable();
                assert_eq!(ids, (0..10).collect::<Vec<_>>());
                let mut rects: Vec<_> = plan.buttons.iter().map(|p| p.1).chain(plan.more).collect();
                rects.sort_by_key(|r| r.0);
                for r in &rects {
                    assert_eq!(r.1, scale(4, dpi));
                    assert!(r.0 >= 0 && r.0 + r.2 <= width);
                }
                for pair in rects.windows(2) {
                    assert!(pair[0].0 + pair[0].2 < pair[1].0);
                }
                let fixed: Vec<_> = plan.buttons.iter().filter(|p| p.0 >= 6).collect();
                assert_eq!(fixed.len(), 4);
                assert_eq!(
                    fixed.last().unwrap().1 .0 + scale(32, dpi),
                    width - scale(4, dpi)
                );
            }
        }
    }
    #[test]
    fn tiny_width_and_empty_input_are_defined() {
        let plan = layout(120, 96, &items());
        assert_eq!(plan.buttons.len(), 4);
        assert_eq!(plan.overflow, vec![0, 1, 2, 3, 4, 5]);
        let empty = layout(1, 96, &[]);
        assert!(empty.buttons.is_empty() && empty.more.is_none());
    }
}
