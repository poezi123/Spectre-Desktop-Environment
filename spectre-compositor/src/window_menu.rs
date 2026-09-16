use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, Size};

pub const WIDTH: i32 = 250;

pub const ITEM_HEIGHT: i32 = 28;

pub const PADDING: i32 = 4;

#[derive(Debug, Clone)]
pub enum MenuFor {
    Window(Window),
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Minimize,
    Maximize,
    Restore,
    Fullscreen,
    SnapLeft,
    SnapRight,
    PreviousWorkspace,
    NextWorkspace,
    Close,
    Terminal,
    Settings,
    Wallpaper,
    ShowDesktop,
    Overview,
}

impl MenuItem {
    pub fn label(self) -> &'static str {
        match self {
            MenuItem::Minimize => "Minimize",
            MenuItem::Maximize => "Maximize",
            MenuItem::Restore => "Restore",
            MenuItem::Fullscreen => "Fullscreen",
            MenuItem::SnapLeft => "Snap to the Left",
            MenuItem::SnapRight => "Snap to the Right",
            MenuItem::PreviousWorkspace => "Move to Previous Workspace",
            MenuItem::NextWorkspace => "Move to Next Workspace",
            MenuItem::Close => "Close",
            MenuItem::Terminal => "Open Terminal",
            MenuItem::Settings => "Settings",
            MenuItem::Wallpaper => "Change Wallpaper",
            MenuItem::ShowDesktop => "Show the Desktop",
            MenuItem::Overview => "Workspace Overview",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowMenu {
    pub target: MenuFor,
    pub items: Vec<MenuItem>,
    pub hovered: Option<usize>,
    origin: Point<i32, Logical>,
}

impl WindowMenu {
    pub fn for_window(
        window: Window,
        at: Point<i32, Logical>,
        maximized: bool,
        area: Rectangle<i32, Logical>,
    ) -> Self {
        Self::new(MenuFor::Window(window), window_items(maximized), at, area)
    }

    pub fn for_desktop(at: Point<i32, Logical>, area: Rectangle<i32, Logical>) -> Self {
        Self::new(MenuFor::Desktop, desktop_items(), at, area)
    }

    fn new(
        target: MenuFor,
        items: Vec<MenuItem>,
        at: Point<i32, Logical>,
        area: Rectangle<i32, Logical>,
    ) -> Self {
        let origin = keep_inside(at, menu_size(items.len()), area);
        Self { target, items, hovered: None, origin }
    }

    pub fn rect(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.origin, menu_size(self.items.len()))
    }

    pub fn item_rect(&self, index: usize) -> Rectangle<i32, Logical> {
        item_rect_at(self.origin, index)
    }

    pub fn item_at(&self, point: Point<f64, Logical>) -> Option<usize> {
        index_at(self.origin, self.items.len(), point)
    }

    pub fn step(&mut self, delta: isize) {
        self.hovered = stepped(self.hovered, delta, self.items.len());
    }
}

fn desktop_items() -> Vec<MenuItem> {
    vec![
        MenuItem::Terminal,
        MenuItem::Settings,
        MenuItem::Wallpaper,
        MenuItem::ShowDesktop,
        MenuItem::Overview,
    ]
}

fn window_items(maximized: bool) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::Minimize];
    if maximized {
        items.push(MenuItem::Restore);
    } else {
        items.push(MenuItem::Maximize);
    }
    items.push(MenuItem::Fullscreen);
    items.push(MenuItem::SnapLeft);
    items.push(MenuItem::SnapRight);
    items.push(MenuItem::PreviousWorkspace);
    items.push(MenuItem::NextWorkspace);
    items.push(MenuItem::Close);
    items
}

fn menu_size(count: usize) -> Size<i32, Logical> {
    Size::from((WIDTH, count as i32 * ITEM_HEIGHT + PADDING * 2))
}

fn item_rect_at(origin: Point<i32, Logical>, index: usize) -> Rectangle<i32, Logical> {
    let location = Point::from((origin.x + PADDING, origin.y + PADDING + index as i32 * ITEM_HEIGHT));
    Rectangle::new(location, Size::from((WIDTH - PADDING * 2, ITEM_HEIGHT)))
}

fn index_at(origin: Point<i32, Logical>, count: usize, point: Point<f64, Logical>) -> Option<usize> {
    let p = Point::<i32, Logical>::from((point.x.floor() as i32, point.y.floor() as i32));
    (0..count).find(|&index| item_rect_at(origin, index).contains(p))
}

fn stepped(hovered: Option<usize>, delta: isize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let count = count as isize;
    let next = match hovered {
        Some(index) => (index as isize + delta).rem_euclid(count),
        None if delta > 0 => 0,
        None => count - 1,
    };
    Some(next as usize)
}

fn keep_inside(
    at: Point<i32, Logical>,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let mut x = at.x;
    let mut y = at.y;
    if x + size.w > area.loc.x + area.size.w {
        x = area.loc.x + area.size.w - size.w;
    }
    if y + size.h > area.loc.y + area.size.h {
        y = area.loc.y + area.size.h - size.h;
    }
    Point::from((x.max(area.loc.x), y.max(area.loc.y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((0, 0)), Size::from((1280, 800)))
    }

    #[test]
    fn a_maximized_window_offers_restore_instead_of_maximize() {
        assert!(window_items(true).contains(&MenuItem::Restore));
        assert!(!window_items(true).contains(&MenuItem::Maximize));
        assert!(window_items(false).contains(&MenuItem::Maximize));
    }

    #[test]
    fn the_desktop_menu_opens_things_rather_than_closing_a_window() {
        let items = desktop_items();
        assert!(items.contains(&MenuItem::Terminal));
        assert!(items.contains(&MenuItem::Settings));
        assert!(!items.contains(&MenuItem::Close));
    }

    #[test]
    fn the_menu_opens_where_it_was_clicked() {
        let size = menu_size(8);
        assert_eq!(keep_inside(Point::from((300, 200)), size, screen()), Point::from((300, 200)));
    }

    #[test]
    fn the_menu_never_hangs_off_the_screen() {
        let size = menu_size(8);
        let origin = keep_inside(Point::from((1200, 780)), size, screen());
        assert_eq!(origin.x + size.w, 1280);
        assert_eq!(origin.y + size.h, 800);
    }

    #[test]
    fn a_point_picks_the_item_under_it() {
        let origin = Point::from((100, 100));
        let y = 100 + PADDING + ITEM_HEIGHT * 3 + ITEM_HEIGHT / 2;
        assert_eq!(index_at(origin, 8, Point::from((150.0, y as f64))), Some(3));
        assert_eq!(index_at(origin, 8, Point::from((20.0, y as f64))), None);
        let below = 100 + PADDING + ITEM_HEIGHT * 8 + 2;
        assert_eq!(index_at(origin, 8, Point::from((150.0, below as f64))), None);
    }

    #[test]
    fn the_arrow_keys_wrap_around() {
        assert_eq!(stepped(None, 1, 8), Some(0));
        assert_eq!(stepped(None, -1, 8), Some(7));
        assert_eq!(stepped(Some(7), 1, 8), Some(0));
        assert_eq!(stepped(Some(0), -1, 8), Some(7));
        assert_eq!(stepped(Some(2), 1, 0), None);
    }
}
