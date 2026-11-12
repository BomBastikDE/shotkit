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
    /// Visual section. A separator is drawn between two neighbouring visible
    /// buttons of different sections; it never affects the overflow order.
    pub section: u8,
}
pub type ButtonRect = (i32, i32, i32, i32);
#[derive(Debug)]
pub struct Layout {
    pub stage: u8,
    pub height: i32,
    pub buttons: Vec<(usize, ButtonRect)>,
    pub overflow: Vec<usize>,
    pub more: Option<ButtonRect>,
    /// Left edge of each separator line; `separator_line` gives its width.
    pub separators: Vec<i32>,
}

/// Additional room a separator needs between two buttons, on top of the gap.
pub fn separator_space(dpi: u32) -> i32 {
    scale(2, dpi)
}

/// Thickness of the separator line itself.
pub fn separator_line(dpi: u32) -> i32 {
    scale(1, dpi)
}

/// Colours of one owner-drawn toolbar button, as Win32 `COLORREF` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonColors {
    pub background: u32,
    pub icon: u32,
    pub frame: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    /// The mouse button is down on this control.
    pub pressed: bool,
    /// The control is the active tool.
    pub checked: bool,
    /// The pointer rests on this control.
    pub hot: bool,
    /// The action currently has no effect (nothing to undo or redo).
    pub dimmed: bool,
}

/// System colours the toolbar draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub face: u32,
    pub text: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub gray_text: u32,
}

/// Blend `weight`/255 of `over` into `base`, per colour channel.
pub fn blend(base: u32, over: u32, weight: u8) -> u32 {
    let weight = u32::from(weight);
    (0..3).fold(0, |color, channel| {
        let shift = channel * 8;
        let from = (base >> shift) & 0xff;
        let to = (over >> shift) & 0xff;
        color | (((from * (255 - weight) + to * weight + 127) / 255) << shift)
    })
}

/// The active tool gets a tinted face with a frame in the accent colour, the
/// button under the pointer a lighter tint, a pressed button a stronger one.
/// High contrast themes keep the plain system selection colours, without
/// blended tints, so every pairing stays one the theme defines.
pub fn button_colors(state: ButtonState, palette: Palette, high_contrast: bool) -> ButtonColors {
    if high_contrast {
        let selected = state.pressed || state.checked;
        return ButtonColors {
            background: if selected {
                palette.accent
            } else {
                palette.face
            },
            icon: if state.dimmed && !selected {
                palette.gray_text
            } else if state.checked {
                palette.accent_text
            } else {
                palette.text
            },
            frame: (state.hot && !selected).then_some(palette.text),
        };
    }
    let tint = if state.pressed {
        96
    } else if state.checked {
        56
    } else if state.hot && !state.dimmed {
        28
    } else {
        0
    };
    ButtonColors {
        background: blend(palette.face, palette.accent, tint),
        icon: if state.dimmed {
            blend(palette.face, palette.text, 96)
        } else {
            palette.text
        },
        frame: state.checked.then_some(palette.accent),
    }
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
    let separator = separator_space(dpi);
    // Separators between the non-fixed buttons accepted by `shown`.
    let separators_among = |shown: &dyn Fn(&Item) -> bool| {
        let mut previous = None;
        let mut count = 0;
        for item in items.iter().filter(|i| i.group != Group::Fixed && shown(i)) {
            if previous.is_some_and(|section| section != item.section) {
                count += 1;
            }
            previous = Some(item.section);
        }
        count
    };
    let all_separators = separators_among(&|_| true) * separator;
    let margin = scale(4, dpi).min(width / (slots * 2 + 2));
    let mut gap = scale(3, dpi).min((width - 2 * margin) / (slots * 2));
    let button_width = if items.len() >= 33 {
        29
    } else if items.len() >= 31 {
        31
    } else {
        32
    };
    let size =
        scale(button_width, dpi).min(((width - 2 * margin - (slots - 1) * gap) / slots).max(1));
    // Preserve full-size controls on a narrow desktop by tightening spacing
    // slightly when that alone keeps every action visible.
    let count = items.len() as i32;
    // Large toolbars use a one-pixel gap and, from 31 items, compact
    // buttons; at 33 items, 29px controls keep every action visible in a
    // 1008px client area (1024px desktop with 16px nonclient width). Smaller
    // toolbars keep their previous spacing and stage boundaries.
    if count >= 30 {
        gap = gap.min(scale(1, dpi));
    } else {
        let compact_gap = scale(2, dpi).min(gap);
        if count > 1
            && 2 * margin + count * size + (count - 1) * compact_gap + all_separators <= width
        {
            gap = gap.min((width - 2 * margin - count * size - all_separators) / (count - 1));
        }
    }
    let top = scale(4, dpi);
    let height = scale(36, dpi);
    let fits = |count: usize, separators: i32| {
        2 * margin + count as i32 * (size + gap) - gap + separators * separator <= width
    };
    let hidden = |group, stage| match group {
        Group::View => stage >= 1,
        Group::SizeHistory => stage >= 2,
        Group::Palette => stage >= 3,
        _ => false,
    };
    let mut stage = 0;
    while stage < 3 {
        let count = items.iter().filter(|i| !hidden(i.group, stage)).count();
        if fits(
            count + usize::from(stage > 0),
            separators_among(&|i| !hidden(i.group, stage)),
        ) {
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
    while !fits(
        visible.len() + usize::from(stage > 0),
        separators_among(&|i| visible.contains(&i.id)),
    ) {
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
    let mut separators = Vec::new();
    let mut left = margin;
    let mut previous = None;
    for item in items
        .iter()
        .filter(|i| i.group != Group::Fixed && visible.contains(&i.id))
    {
        if previous.is_some_and(|section| section != item.section) {
            // Centre the line in the widened space between the two buttons.
            separators.push(left - gap + (gap + separator - separator_line(dpi)).max(0) / 2);
            left += separator;
        }
        previous = Some(item.section);
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
        separators,
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
            .map(|(id, group)| Item {
                id,
                group,
                section: 0,
            })
            .collect()
    }
    #[test]
    fn stages_move_groups_in_reference_order_and_restore_on_growth() {
        let items = items();
        for (width, stage, hidden) in [
            (346, 0, vec![]),
            (345, 1, vec![4, 5]),
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

    #[test]
    fn compact_spacing_keeps_full_size_buttons_on_a_1024_pixel_desktop() {
        let items: Vec<_> = (0..30)
            .map(|id| Item {
                id,
                group: if id >= 26 { Group::Fixed } else { Group::Tool },
                section: 0,
            })
            .collect();
        let plan = layout(1008, 96, &items);
        assert_eq!(plan.stage, 0);
        assert!(plan.overflow.is_empty());
        assert!(plan.buttons.iter().all(|(_, rect)| rect.2 == 32));
        let mut rects: Vec<_> = plan.buttons.iter().map(|entry| entry.1).collect();
        rects.sort_by_key(|rect| rect.0);
        assert!(rects
            .windows(2)
            .all(|pair| pair[0].0 + pair[0].2 < pair[1].0));
        assert!(!layout(996, 96, &items).overflow.is_empty());
    }

    #[test]
    fn thirty_three_actions_fit_a_1024_pixel_desktop_client_area() {
        let items: Vec<_> = (0..33)
            .map(|id| Item {
                id,
                group: if id >= 29 { Group::Fixed } else { Group::Tool },
                section: 0,
            })
            .collect();
        let plan = layout(1008, 96, &items);
        assert_eq!(plan.stage, 0);
        assert!(plan.overflow.is_empty() && plan.more.is_none());
        assert_eq!(plan.buttons.len(), 33);
        assert!(plan.buttons.iter().all(|(_, rect)| rect.2 == 29));
        let mut rects: Vec<_> = plan.buttons.iter().map(|entry| entry.1).collect();
        rects.sort_by_key(|rect| rect.0);
        assert!(rects
            .iter()
            .all(|rect| rect.0 >= 0 && rect.0 + rect.2 <= 1008));
        assert!(rects
            .windows(2)
            .all(|pair| pair[0].0 + pair[0].2 < pair[1].0));
    }

    fn sectioned(sections: &[u8], fixed: usize) -> Vec<Item> {
        sections
            .iter()
            .map(|section| (Group::Tool, *section))
            .chain((0..fixed).map(|_| (Group::Fixed, 0)))
            .enumerate()
            .map(|(id, (group, section))| Item { id, group, section })
            .collect()
    }

    #[test]
    fn separators_sit_between_sections_and_never_touch_buttons() {
        for dpi in [96, 120, 144, 192] {
            let items = sectioned(&[0, 0, 1, 1, 1, 2], 2);
            let plan = layout(scale(600, dpi), dpi, &items);
            assert!(plan.overflow.is_empty());
            assert_eq!(plan.separators.len(), 2, "dpi {dpi}");
            let rect = |id: usize| plan.buttons.iter().find(|b| b.0 == id).unwrap().1;
            for (line, (before, after)) in plan.separators.iter().zip([(1, 2), (4, 5)]) {
                let (before, after) = (rect(before), rect(after));
                assert!(before.0 + before.2 <= *line, "dpi {dpi}");
                assert!(line + separator_line(dpi) <= after.0, "dpi {dpi}");
            }
            // Buttons of one section keep the plain gap.
            assert_eq!(rect(1).0 - rect(0).0, rect(3).0 - rect(2).0);
            assert!(rect(2).0 - rect(1).0 > rect(1).0 - rect(0).0);
        }
    }

    #[test]
    fn separators_count_towards_the_available_width() {
        let items = sectioned(&[0, 0, 1, 1], 1);
        let plain = sectioned(&[0, 0, 0, 0], 1);
        let needed = |items: &[Item]| {
            (1..2000)
                .find(|width| layout(*width, 96, items).overflow.is_empty())
                .unwrap()
        };
        assert_eq!(needed(&items), needed(&plain) + separator_space(96));
        // A hidden section takes its separator with it.
        let narrow = layout(needed(&items) - 1, 96, &items);
        assert!(!narrow.overflow.is_empty());
        assert!(narrow.separators.len() <= 1);
        // No separator in front of the first button or the overflow button.
        let single = layout(needed(&items), 96, &sectioned(&[3, 3, 3], 1));
        assert!(single.separators.is_empty());
    }

    const PALETTE: Palette = Palette {
        face: 0x00F0_F0F0,
        text: 0x0000_0000,
        accent: 0x00D7_7800,
        accent_text: 0x00FF_FFFF,
        gray_text: 0x006D_6D6D,
    };

    #[test]
    fn blend_interpolates_each_channel() {
        assert_eq!(blend(0x0010_2030, 0x00F0_E0D0, 0), 0x0010_2030);
        assert_eq!(blend(0x0010_2030, 0x00F0_E0D0, 255), 0x00F0_E0D0);
        assert_eq!(blend(0x0000_0000, 0x00FF_FFFF, 128), 0x0080_8080);
    }

    #[test]
    fn button_states_are_visually_distinct() {
        let colors = |state| button_colors(state, PALETTE, false);
        let idle = colors(ButtonState::default());
        let hot = colors(ButtonState {
            hot: true,
            ..Default::default()
        });
        let checked = colors(ButtonState {
            checked: true,
            ..Default::default()
        });
        let pressed = colors(ButtonState {
            pressed: true,
            ..Default::default()
        });
        let dimmed = colors(ButtonState {
            dimmed: true,
            hot: true,
            ..Default::default()
        });
        assert_eq!(idle.background, PALETTE.face);
        assert_eq!((idle.icon, idle.frame), (PALETTE.text, None));
        let backgrounds = [
            idle.background,
            hot.background,
            checked.background,
            pressed.background,
        ];
        for (index, color) in backgrounds.iter().enumerate() {
            assert!(!backgrounds[..index].contains(color));
        }
        assert_eq!(checked.frame, Some(PALETTE.accent));
        assert_eq!(hot.frame, None);
        // An action without effect does not react to the pointer and is paler.
        assert_eq!(dimmed.background, PALETTE.face);
        assert!(dimmed.icon != PALETTE.text && dimmed.icon != PALETTE.face);
    }

    #[test]
    fn high_contrast_uses_only_system_colours() {
        let colors = |state| button_colors(state, PALETTE, true);
        let checked = colors(ButtonState {
            checked: true,
            ..Default::default()
        });
        assert_eq!(
            (checked.background, checked.icon),
            (PALETTE.accent, PALETTE.accent_text)
        );
        let hot = colors(ButtonState {
            hot: true,
            ..Default::default()
        });
        assert_eq!(
            (hot.background, hot.icon, hot.frame),
            (PALETTE.face, PALETTE.text, Some(PALETTE.text))
        );
        let dimmed = colors(ButtonState {
            dimmed: true,
            ..Default::default()
        });
        assert_eq!(dimmed.icon, PALETTE.gray_text);
        let system = [
            PALETTE.face,
            PALETTE.text,
            PALETTE.accent,
            PALETTE.accent_text,
            PALETTE.gray_text,
        ];
        for state in [checked, hot, dimmed, colors(ButtonState::default())] {
            assert!(system.contains(&state.background) && system.contains(&state.icon));
        }
    }
}
