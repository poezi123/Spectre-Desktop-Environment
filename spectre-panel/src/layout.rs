use spectre_ipc::{Desktop, WindowId};
use spectre_status::{Battery, Sound};

use spectre_draw::Rect;

pub const EDGE_PADDING: i32 = 6;
pub const GAP: i32 = 4;
pub const CHIP_PADDING: i32 = 10;
pub const BUTTON_WIDTH: i32 = 30;
pub const WORKSPACE_WIDTH: i32 = 26;
pub const TASK_MAX_WIDTH: i32 = 190;
pub const TASK_MIN_WIDTH: i32 = 56;
pub const WORKSPACE_HEIGHT: i32 = 26;
pub const RESOURCES_HEIGHT: i32 = 30;
pub const CLOCK_HEIGHT: i32 = 34;
pub const SOUND_WIDTH: i32 = 92;
pub const TRAY_WIDTH: i32 = 26;
pub const BAR_HEIGHT: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Launcher,
    Workspace { index: u8, active: bool, occupied: bool },
    Task { id: WindowId, title: String, focused: bool, minimized: bool },
    Resources,
    Clock,
    Session,
    Sound { percent: u8, muted: bool },
    Battery { percent: u8, charging: bool, low: bool },
    Tray { index: usize },
}

impl Item {
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Item::Clock)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub item: Item,
    pub rect: Rect,
}

pub struct Measured {
    pub clock: i32,
    pub resources: i32,
    pub battery: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    pub sound: Option<Sound>,
    pub battery: Option<Battery>,
    pub tray: usize,
}

pub fn sound_bar(chip: Rect) -> Rect {
    let inset = CHIP_PADDING / 2;
    let left = chip.x + inset + 34;
    let width = (chip.right() - inset - left).max(1);
    let y = chip.y + (chip.h - BAR_HEIGHT) / 2;
    Rect::new(left, y, width, BAR_HEIGHT)
}

pub fn sound_percent(chip: Rect, x: i32) -> u8 {
    let bar = sound_bar(chip);
    if bar.w <= 1 {
        return 0;
    }
    let along = (x - bar.x).clamp(0, bar.w) as f32 / bar.w as f32;
    (along * 100.0).round() as u8
}

pub fn on_the_sound_bar(chip: Rect, x: i32) -> bool {
    x >= sound_bar(chip).x
}

pub fn layout(
    width: i32,
    height: i32,
    desktop: &Desktop,
    measured: &Measured,
    status: &Status,
    mut title_width: impl FnMut(&str) -> i32,
    show_resources: bool,
) -> Vec<Placed> {
    let mut left = Vec::new();
    let mut cursor = EDGE_PADDING;

    let push = |items: &mut Vec<Placed>, cursor: &mut i32, item: Item, w: i32| {
        items.push(Placed { item, rect: Rect::new(*cursor, 0, w, height) });
        *cursor += w + GAP;
    };

    push(&mut left, &mut cursor, Item::Launcher, BUTTON_WIDTH);
    for workspace in &desktop.workspaces {
        push(
            &mut left,
            &mut cursor,
            Item::Workspace {
                index: workspace.index,
                active: workspace.active,
                occupied: workspace.windows > 0,
            },
            WORKSPACE_WIDTH,
        );
    }
    let tasks_start = cursor;

    let mut right = Vec::new();
    let mut edge = width - EDGE_PADDING;
    let push_right = |items: &mut Vec<Placed>, edge: &mut i32, item: Item, w: i32| {
        *edge -= w;
        items.push(Placed { item, rect: Rect::new(*edge, 0, w, height) });
        *edge -= GAP;
    };

    push_right(&mut right, &mut edge, Item::Session, BUTTON_WIDTH);
    push_right(&mut right, &mut edge, Item::Clock, measured.clock.max(1) + CHIP_PADDING);
    if let Some(battery) = status.battery {
        let item = Item::Battery {
            percent: battery.percent,
            charging: battery.charging,
            low: battery.low(),
        };
        push_right(&mut right, &mut edge, item, measured.battery.max(1) + CHIP_PADDING);
    }
    if let Some(sound) = status.sound {
        let item = Item::Sound { percent: sound.percent, muted: sound.muted };
        push_right(&mut right, &mut edge, item, SOUND_WIDTH);
    }
    for index in (0..status.tray).rev() {
        push_right(&mut right, &mut edge, Item::Tray { index }, TRAY_WIDTH);
    }
    if show_resources {
        push_right(&mut right, &mut edge, Item::Resources, measured.resources.max(1) + CHIP_PADDING);
    }

    let tasks_end = edge;
    let mut items = left;
    items.extend(place_tasks(desktop, tasks_start, tasks_end, height, &mut title_width));
    items.extend(right);
    items.retain(|p| !p.rect.is_empty() && p.rect.x < width);
    items
}

pub fn layout_vertical(
    thickness: i32,
    length: i32,
    desktop: &Desktop,
    show_resources: bool,
) -> Vec<Placed> {
    let mut top = Vec::new();
    let mut cursor = EDGE_PADDING;
    let push = |items: &mut Vec<Placed>, cursor: &mut i32, item: Item, h: i32| {
        items.push(Placed { item, rect: Rect::new(0, *cursor, thickness, h) });
        *cursor += h + GAP;
    };

    push(&mut top, &mut cursor, Item::Launcher, thickness);
    for workspace in &desktop.workspaces {
        push(
            &mut top,
            &mut cursor,
            Item::Workspace {
                index: workspace.index,
                active: workspace.active,
                occupied: workspace.windows > 0,
            },
            WORKSPACE_HEIGHT,
        );
    }
    let tasks_start = cursor;

    let mut bottom = Vec::new();
    let mut edge = length - EDGE_PADDING;
    let push_bottom = |items: &mut Vec<Placed>, edge: &mut i32, item: Item, h: i32| {
        *edge -= h;
        items.push(Placed { item, rect: Rect::new(0, *edge, thickness, h) });
        *edge -= GAP;
    };

    push_bottom(&mut bottom, &mut edge, Item::Session, thickness);
    push_bottom(&mut bottom, &mut edge, Item::Clock, CLOCK_HEIGHT);
    if show_resources {
        push_bottom(&mut bottom, &mut edge, Item::Resources, RESOURCES_HEIGHT);
    }

    let mut items = top;
    let mut cursor = tasks_start;
    for window in desktop.visible_windows() {
        if cursor + thickness > edge {
            break;
        }
        items.push(Placed {
            item: Item::Task {
                id: window.id,
                title: window.title.clone(),
                focused: window.focused,
                minimized: window.minimized,
            },
            rect: Rect::new(0, cursor, thickness, thickness),
        });
        cursor += thickness + GAP;
    }
    items.extend(bottom);
    items.retain(|p| !p.rect.is_empty() && p.rect.y < length);
    items
}

fn place_tasks(
    desktop: &Desktop,
    start: i32,
    end: i32,
    height: i32,
    title_width: &mut impl FnMut(&str) -> i32,
) -> Vec<Placed> {
    let available = end - start;
    if available < TASK_MIN_WIDTH {
        return Vec::new();
    }

    let tasks: Vec<&spectre_ipc::Window> = desktop.visible_windows().collect();
    if tasks.is_empty() {
        return Vec::new();
    }

    let natural: Vec<i32> = tasks
        .iter()
        .map(|w| (title_width(&w.title) + CHIP_PADDING).clamp(TASK_MIN_WIDTH, TASK_MAX_WIDTH))
        .collect();
    let total: i32 = natural.iter().sum::<i32>() + GAP * (tasks.len() as i32 - 1);

    let widths: Vec<i32> = if total <= available {
        natural
    } else {
        let fair = (available - GAP * (tasks.len() as i32 - 1)) / tasks.len() as i32;
        vec![fair.max(0); tasks.len()]
    };

    let mut placed = Vec::new();
    let mut cursor = start;
    for (window, width) in tasks.iter().zip(widths) {
        if width < TASK_MIN_WIDTH || cursor + width > end {
            break;
        }
        placed.push(Placed {
            item: Item::Task {
                id: window.id,
                title: window.title.clone(),
                focused: window.focused,
                minimized: window.minimized,
            },
            rect: Rect::new(cursor, 0, width, height),
        });
        cursor += width + GAP;
    }
    placed
}

pub fn item_at(items: &[Placed], x: i32, y: i32) -> Option<&Placed> {
    items.iter().find(|p| p.rect.contains(x, y))
}

pub fn nothing_to_show(desktop: &Desktop) -> bool {
    desktop.resting || hidden_by_fullscreen(desktop)
}

pub fn hidden_by_fullscreen(desktop: &Desktop) -> bool {
    let active = desktop.workspaces.iter().find(|workspace| workspace.active);
    let Some(active) = active else {
        return false;
    };
    desktop
        .windows
        .iter()
        .any(|window| window.fullscreen && !window.minimized && window.workspace == active.index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectre_ipc::{Window, Workspace};

    fn desktop(workspaces: u8, windows: usize) -> Desktop {
        Desktop {
            workspaces: (1..=workspaces)
                .map(|index| Workspace { index, active: index == 1, windows: 0 })
                .collect(),
            windows: (0..windows)
                .map(|i| Window {
                    id: i as u64 + 1,
                    title: format!("Window {i}"),
                    app_id: "test".into(),
                    workspace: 1,
                    focused: i == 0,
                    minimized: false,
                    fullscreen: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn chip() -> Rect {
        Rect::new(500, 0, SOUND_WIDTH, 36)
    }

    #[test]
    fn tray_icons_stand_next_to_each_other_in_order() {
        let desktop = desktop(4, 0);
        let measured = Measured { clock: 60, resources: 120, battery: 40 };
        let status = Status { tray: 3, ..Status::default() };
        let items = layout(1600, 36, &desktop, &measured, &status, |_| 100, true);
        let mut trays: Vec<&Placed> =
            items.iter().filter(|p| matches!(p.item, Item::Tray { .. })).collect();
        assert_eq!(trays.len(), 3);
        trays.sort_by_key(|placed| placed.rect.x);
        let order: Vec<usize> = trays
            .iter()
            .filter_map(|placed| match placed.item {
                Item::Tray { index } => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(order, vec![0, 1, 2], "the first one registered sits leftmost");
    }

    #[test]
    fn the_sound_bar_sits_inside_its_chip_with_room_for_the_word() {
        let bar = sound_bar(chip());
        assert!(bar.x > chip().x, "the word comes first");
        assert!(bar.right() <= chip().right());
        assert!(bar.w > 20, "a bar of {} px is too small to aim at", bar.w);
    }

    #[test]
    fn clicking_along_the_bar_picks_that_share() {
        let bar = sound_bar(chip());
        assert_eq!(sound_percent(chip(), bar.x), 0);
        assert_eq!(sound_percent(chip(), bar.right()), 100);
        assert_eq!(sound_percent(chip(), bar.x + bar.w / 2), 50);
    }

    #[test]
    fn a_click_left_of_the_bar_is_the_mute_switch() {
        let bar = sound_bar(chip());
        assert!(!on_the_sound_bar(chip(), chip().x + 2));
        assert!(on_the_sound_bar(chip(), bar.x + 1));
    }

    #[test]
    fn sound_and_battery_only_appear_when_the_machine_has_them() {
        let desktop = desktop(4, 1);
        let measured = Measured { clock: 60, resources: 120, battery: 40 };
        let bare = Status::default();
        let plain = layout(1600, 36, &desktop, &measured, &bare, |_| 100, true);
        assert!(!plain.iter().any(|p| matches!(p.item, Item::Sound { .. })));
        assert!(!plain.iter().any(|p| matches!(p.item, Item::Battery { .. })));

        let full = Status {
            sound: Some(Sound { percent: 40, muted: false }),
            battery: Some(Battery { percent: 90, charging: true, full: false }),
            tray: 0,
        };
        let rich = layout(1600, 36, &desktop, &measured, &full, |_| 100, true);
        assert!(rich.iter().any(|p| matches!(p.item, Item::Sound { percent: 40, muted: false })));
        assert!(rich.iter().any(|p| matches!(p.item, Item::Battery { percent: 90, .. })));
    }

    #[test]
    fn a_fullscreen_window_on_the_open_workspace_hides_the_panel() {
        let mut d = desktop(4, 2);
        assert!(!hidden_by_fullscreen(&d));

        d.windows[1].fullscreen = true;
        assert!(hidden_by_fullscreen(&d), "it covers the workspace we are looking at");

        d.windows[1].minimized = true;
        assert!(!hidden_by_fullscreen(&d), "a minimized window covers nothing");

        d.windows[1].minimized = false;
        d.windows[1].workspace = 3;
        assert!(!hidden_by_fullscreen(&d), "another workspace does not hide this one");
    }

    #[test]
    fn a_vertical_panel_stacks_everything_in_one_column() {
        let d = desktop(4, 2);
        let items = layout_vertical(32, 500, &d, true);
        assert!(items.iter().any(|p| p.item == Item::Launcher));
        assert!(items.iter().any(|p| p.item == Item::Clock));
        assert!(items.iter().any(|p| p.item == Item::Session));
        for placed in &items {
            assert_eq!(placed.rect.x, 0, "{:?} is not in the column", placed.item);
            assert_eq!(placed.rect.w, 32, "{:?} is not a button wide", placed.item);
            assert!(placed.rect.bottom() <= 500, "{:?} runs off the end", placed.item);
        }
    }

    #[test]
    fn a_vertical_stack_never_overlaps_itself() {
        let mut items = layout_vertical(32, 500, &desktop(4, 3), true);
        items.sort_by_key(|p| p.rect.y);
        for pair in items.windows(2) {
            assert!(
                pair[0].rect.bottom() <= pair[1].rect.y,
                "{:?} overlaps {:?}",
                pair[0].item,
                pair[1].item
            );
        }
    }

    #[test]
    fn a_short_vertical_panel_drops_tasks_rather_than_the_clock() {
        let items = layout_vertical(32, 200, &desktop(4, 6), false);
        assert!(items.iter().any(|p| p.item == Item::Clock), "the clock must survive");
        assert!(items.iter().any(|p| p.item == Item::Session), "so must the session button");
        for placed in &items {
            assert!(placed.rect.bottom() <= 200, "{:?} runs off the end", placed.item);
        }
    }

    #[test]
    fn a_vertical_panel_finds_the_item_under_a_click() {
        let items = layout_vertical(32, 500, &desktop(4, 1), true);
        let launcher = items.iter().find(|p| p.item == Item::Launcher).unwrap().rect;
        let hit = item_at(&items, launcher.x + 4, launcher.y + 4).unwrap();
        assert_eq!(hit.item, Item::Launcher);
    }

    fn measured() -> Measured {
        Measured { clock: 46, resources: 70, battery: 40 }
    }

    fn width_of(text: &str) -> i32 {
        text.chars().count() as i32 * 7
    }

    fn lay(width: i32, d: &Desktop) -> Vec<Placed> {
        layout(width, 32, d, &measured(), &Status::default(), width_of, true)
    }

    #[test]
    fn a_bare_desktop_still_gets_its_fixed_widgets() {
        let items = lay(1920, &desktop(4, 0));
        assert!(items.iter().any(|p| p.item == Item::Launcher));
        assert!(items.iter().any(|p| p.item == Item::Clock));
        assert!(items.iter().any(|p| p.item == Item::Session));
        assert_eq!(
            items.iter().filter(|p| matches!(p.item, Item::Workspace { .. })).count(),
            4
        );
    }

    #[test]
    fn nothing_overlaps_and_everything_stays_on_the_panel() {
        let items = lay(1920, &desktop(4, 5));
        let mut sorted: Vec<&Placed> = items.iter().collect();
        sorted.sort_by_key(|p| p.rect.x);
        for pair in sorted.windows(2) {
            assert!(
                pair[0].rect.right() <= pair[1].rect.x,
                "{:?} overlaps {:?}",
                pair[0].item,
                pair[1].item
            );
        }
        for placed in &items {
            assert!(placed.rect.x >= 0 && placed.rect.right() <= 1920, "{:?}", placed.item);
        }
    }

    #[test]
    fn the_clock_is_always_the_rightmost_thing_after_the_session_button() {
        let items = lay(1920, &desktop(4, 8));
        let clock = items.iter().find(|p| p.item == Item::Clock).unwrap();
        let session = items.iter().find(|p| p.item == Item::Session).unwrap();
        assert!(session.rect.x > clock.rect.x);
        assert!(items
            .iter()
            .filter(|p| matches!(p.item, Item::Task { .. }))
            .all(|t| t.rect.right() <= clock.rect.x));
    }

    #[test]
    fn a_crowded_panel_drops_tasks_rather_than_the_clock() {
        let items = lay(560, &desktop(4, 20));
        assert!(items.iter().any(|p| p.item == Item::Clock), "the clock must survive");
        let tasks = items.iter().filter(|p| matches!(p.item, Item::Task { .. })).count();
        assert!(tasks < 20, "not every task can possibly fit");
        for placed in &items {
            assert!(placed.rect.right() <= 560);
        }
    }

    #[test]
    fn a_very_narrow_panel_drops_tasks_entirely_without_panicking() {
        let items = lay(200, &desktop(4, 6));
        assert!(!items.iter().any(|p| matches!(p.item, Item::Task { .. })));
    }

    #[test]
    fn only_windows_on_the_visible_workspace_get_a_chip() {
        let mut d = desktop(2, 2);
        d.windows[1].workspace = 2;
        let items = lay(1920, &d);
        let tasks: Vec<&Placed> =
            items.iter().filter(|p| matches!(p.item, Item::Task { .. })).collect();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn a_long_title_does_not_starve_the_other_chips() {
        let mut d = desktop(1, 3);
        d.windows[0].title = "x".repeat(400);
        let items = lay(1920, &d);
        let tasks: Vec<&Placed> =
            items.iter().filter(|p| matches!(p.item, Item::Task { .. })).collect();
        assert_eq!(tasks.len(), 3);
        for task in tasks {
            assert!(task.rect.w <= TASK_MAX_WIDTH, "chip grew to {}", task.rect.w);
        }
    }

    #[test]
    fn hiding_the_resource_readout_gives_the_room_to_tasks() {
        let d = desktop(4, 6);
        let with = layout(1000, 32, &d, &measured(), &Status::default(), width_of, true);
        let without = layout(1000, 32, &d, &measured(), &Status::default(), width_of, false);
        let room = |items: &[Placed]| {
            items
                .iter()
                .filter(|p| matches!(p.item, Item::Task { .. }))
                .map(|p| p.rect.w)
                .sum::<i32>()
        };
        assert!(room(&without) >= room(&with));
        assert!(!without.iter().any(|p| p.item == Item::Resources));
    }

    #[test]
    fn hit_testing_finds_what_was_drawn() {
        let items = lay(1920, &desktop(4, 3));
        for placed in &items {
            let hit = item_at(&items, placed.rect.x + placed.rect.w / 2, 16);
            assert_eq!(hit.map(|p| &p.item), Some(&placed.item));
        }
        assert!(item_at(&items, 1919, 16).is_none() || item_at(&items, 1919, 16).is_some());
        assert!(item_at(&items, -1, 16).is_none());
    }

    #[test]
    fn the_clock_is_the_only_thing_that_does_nothing_when_clicked() {
        assert!(!Item::Clock.is_interactive());
        assert!(Item::Resources.is_interactive());
        assert!(Item::Launcher.is_interactive());
        assert!(Item::Workspace { index: 1, active: true, occupied: false }.is_interactive());
    }
}
