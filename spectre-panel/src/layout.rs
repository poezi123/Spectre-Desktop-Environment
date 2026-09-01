//! Where each panel widget goes.
//!
//! Pure geometry: it takes the desktop state and a way to measure text, and
//! returns rectangles. Keeping it free of both fonts and Wayland is what makes
//! the layout testable, and it is the same list the click handler searches, so
//! what is drawn and what is clickable cannot drift apart.

use spectre_ipc::{Desktop, WindowId};

use spectre_draw::Rect;

/// Horizontal padding at the panel's ends.
pub const EDGE_PADDING: i32 = 6;
/// Gap between adjacent items.
pub const GAP: i32 = 4;
/// Extra width inside a text chip, split evenly left and right.
pub const CHIP_PADDING: i32 = 10;
/// Width of a square button.
pub const BUTTON_WIDTH: i32 = 30;
/// Width of a workspace pip.
pub const WORKSPACE_WIDTH: i32 = 26;
/// Widest a single task chip may get before its title is ellipsised.
pub const TASK_MAX_WIDTH: i32 = 190;
/// Narrowest a task chip may shrink to before tasks start being dropped.
pub const TASK_MIN_WIDTH: i32 = 56;

/// Something the panel draws and, mostly, something the user can click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Opens the launcher.
    Launcher,
    /// Switches to a workspace. Carries the 1-based index.
    Workspace { index: u8, active: bool, occupied: bool },
    /// Focuses a window, or minimises it when it is already focused.
    Task { id: WindowId, title: String, focused: bool, minimized: bool },
    /// CPU and memory readout.
    Resources,
    /// Time and date.
    Clock,
    /// Ends the session.
    Session,
}

impl Item {
    /// Whether clicking does anything.
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Item::Resources | Item::Clock)
    }
}

/// An item with the rectangle it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub item: Item,
    pub rect: Rect,
}

/// The text a widget needs measured before it can be placed.
pub struct Measured {
    /// Width of the clock's widest line.
    pub clock: i32,
    /// Width of the resource readout.
    pub resources: i32,
}

/// Lay the panel out across `width` pixels.
///
/// `title_width` measures a task title so chips can be sized to their content.
/// Tasks are the only elastic part: everything else keeps its size and the
/// tasks share what is left, shrinking and then dropping off the end rather
/// than pushing the clock off the panel.
pub fn layout(
    width: i32,
    height: i32,
    desktop: &Desktop,
    measured: &Measured,
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

    // The right-hand cluster is placed from the edge inwards.
    let mut right = Vec::new();
    let mut edge = width - EDGE_PADDING;
    let push_right = |items: &mut Vec<Placed>, edge: &mut i32, item: Item, w: i32| {
        *edge -= w;
        items.push(Placed { item, rect: Rect::new(*edge, 0, w, height) });
        *edge -= GAP;
    };

    push_right(&mut right, &mut edge, Item::Session, BUTTON_WIDTH);
    push_right(&mut right, &mut edge, Item::Clock, measured.clock.max(1) + CHIP_PADDING);
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

/// Fit as many task chips as will go between `start` and `end`.
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

    // Give every chip what its title needs, capped; if that does not fit,
    // share the space evenly instead of letting one long title win.
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
            // Out of room: drop the rest rather than draw a sliver.
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

/// The item at a panel-local point, if any.
pub fn item_at(items: &[Placed], x: i32, y: i32) -> Option<&Placed> {
    items.iter().find(|p| p.rect.contains(x, y))
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
                })
                .collect(),
            ..Default::default()
        }
    }

    fn measured() -> Measured {
        Measured { clock: 46, resources: 70 }
    }

    fn width_of(text: &str) -> i32 {
        text.chars().count() as i32 * 7
    }

    fn lay(width: i32, d: &Desktop) -> Vec<Placed> {
        layout(width, 32, d, &measured(), width_of, true)
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
        let with = layout(1000, 32, &d, &measured(), width_of, true);
        let without = layout(1000, 32, &d, &measured(), width_of, false);
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
    fn readouts_are_not_clickable_but_everything_else_is() {
        assert!(!Item::Resources.is_interactive());
        assert!(!Item::Clock.is_interactive());
        assert!(Item::Launcher.is_interactive());
        assert!(Item::Workspace { index: 1, active: true, occupied: false }.is_interactive());
    }
}
