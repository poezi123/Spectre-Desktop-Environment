use smithay::utils::{Logical, Point, Rectangle, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    start: Option<Point<i32, Logical>>,
    at: Point<i32, Logical>,
}

impl Pick {
    pub fn new(at: Point<i32, Logical>) -> Self {
        Self { start: None, at }
    }

    pub fn begin(&mut self, at: Point<i32, Logical>) {
        self.start = Some(at);
        self.at = at;
    }

    pub fn moved(&mut self, at: Point<i32, Logical>) {
        self.at = at;
    }

    pub fn area(&self) -> Option<Rectangle<i32, Logical>> {
        let start = self.start?;
        let area = between(start, self.at);
        if area.size.w < 2 || area.size.h < 2 {
            return None;
        }
        Some(area)
    }
}

pub fn between(one: Point<i32, Logical>, other: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    let corner = Point::from((one.x.min(other.x), one.y.min(other.y)));
    let size = Size::from(((one.x - other.x).abs(), (one.y - other.y).abs()));
    Rectangle::new(corner, size)
}

pub fn around(
    area: Rectangle<i32, Logical>,
    screen: Rectangle<i32, Logical>,
) -> Vec<Rectangle<i32, Logical>> {
    let Some(area) = area.intersection(screen) else {
        return vec![screen];
    };
    let mut bands = Vec::new();
    let above = area.loc.y - screen.loc.y;
    if above > 0 {
        bands.push(Rectangle::new(screen.loc, Size::from((screen.size.w, above))));
    }
    let below = screen.loc.y + screen.size.h - (area.loc.y + area.size.h);
    if below > 0 {
        let corner = Point::from((screen.loc.x, area.loc.y + area.size.h));
        bands.push(Rectangle::new(corner, Size::from((screen.size.w, below))));
    }
    let left = area.loc.x - screen.loc.x;
    if left > 0 {
        let corner = Point::from((screen.loc.x, area.loc.y));
        bands.push(Rectangle::new(corner, Size::from((left, area.size.h))));
    }
    let right = screen.loc.x + screen.size.w - (area.loc.x + area.size.w);
    if right > 0 {
        let corner = Point::from((area.loc.x + area.size.w, area.loc.y));
        bands.push(Rectangle::new(corner, Size::from((right, area.size.h))));
    }
    bands
}

pub fn edges(area: Rectangle<i32, Logical>, width: i32) -> Vec<Rectangle<i32, Logical>> {
    let width = width.max(1);
    if area.size.w <= 0 || area.size.h <= 0 {
        return Vec::new();
    }
    let top = Rectangle::new(area.loc, Size::from((area.size.w, width)));
    let bottom = Rectangle::new(
        Point::from((area.loc.x, area.loc.y + area.size.h - width)),
        Size::from((area.size.w, width)),
    );
    let left = Rectangle::new(area.loc, Size::from((width, area.size.h)));
    let right = Rectangle::new(
        Point::from((area.loc.x + area.size.w - width, area.loc.y)),
        Size::from((width, area.size.h)),
    );
    vec![top, bottom, left, right]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> Point<i32, Logical> {
        Point::from((x, y))
    }

    fn screen() -> Rectangle<i32, Logical> {
        Rectangle::new(point(0, 0), Size::from((1920, 1080)))
    }

    #[test]
    fn a_drag_in_any_direction_gives_the_same_rectangle() {
        let one = between(point(100, 100), point(300, 250));
        let other = between(point(300, 250), point(100, 100));
        assert_eq!(one, other);
        assert_eq!(one.loc, point(100, 100));
        assert_eq!(one.size, Size::from((200, 150)));
    }

    #[test]
    fn a_pick_is_only_an_area_once_it_has_some_size() {
        let mut pick = Pick::new(point(10, 10));
        assert_eq!(pick.area(), None, "nothing has been dragged yet");

        pick.begin(point(10, 10));
        assert_eq!(pick.area(), None, "a single point is not an area");

        pick.moved(point(60, 40));
        assert_eq!(pick.area(), Some(Rectangle::new(point(10, 10), Size::from((50, 30)))));
    }

    #[test]
    fn the_bands_cover_the_screen_without_the_picked_area() {
        let area = Rectangle::new(point(200, 100), Size::from((400, 300)));
        let bands = around(area, screen());
        let covered: i32 = bands.iter().map(|band| band.size.w * band.size.h).sum();
        let whole = screen().size.w * screen().size.h;
        assert_eq!(covered, whole - area.size.w * area.size.h);
        for band in &bands {
            assert!(band.intersection(area).is_none(), "{band:?} overlaps the picked area");
        }
    }

    #[test]
    fn an_area_in_the_corner_leaves_only_two_bands() {
        let area = Rectangle::new(point(0, 0), Size::from((500, 400)));
        assert_eq!(around(area, screen()).len(), 2);
    }

    #[test]
    fn an_area_outside_the_screen_dims_everything() {
        let far = Rectangle::new(point(4000, 4000), Size::from((100, 100)));
        assert_eq!(around(far, screen()), vec![screen()]);
    }

    #[test]
    fn the_four_edges_stay_inside_the_area() {
        let area = Rectangle::new(point(50, 50), Size::from((100, 80)));
        let lines = edges(area, 2);
        assert_eq!(lines.len(), 4);
        for line in lines {
            assert_eq!(line.intersection(area), Some(line), "{line:?} sticks out");
        }
    }
}
