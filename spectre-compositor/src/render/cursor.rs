use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Size, Transform};
use spectre_theme::Color;

const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 15.0),
    (3.4, 11.4),
    (6.4, 18.6),
    (8.8, 17.6),
    (6.6, 10.9),
    (10.6, 10.9),
];

const ART_HEIGHT: f32 = 19.0;

const OUTLINE_RATIO: f32 = 1.0 / 14.0;

pub struct CursorImage {
    pub buffer: MemoryRenderBuffer,
    pub hotspot: (i32, i32),
    pub size: (i32, i32),
}

impl std::fmt::Debug for CursorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorImage").field("size", &self.size).finish()
    }
}

impl CursorImage {
    pub fn new(height: i32, fill: Color, outline: Color) -> Self {
        let art = Art::new(height);
        let pixels = art.rasterise(fill, outline);
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            Size::from((art.width, art.height)),
            1,
            Transform::Normal,
            None,
        );
        Self { buffer, hotspot: (art.pad, art.pad), size: (art.width, art.height) }
    }
}

struct Art {
    points: [(f32, f32); ARROW.len()],
    outline: f32,
    pad: i32,
    width: i32,
    height: i32,
}

impl Art {
    fn new(height: i32) -> Self {
        let height = height.max(6);
        let scale = height as f32 / ART_HEIGHT;
        let outline = (height as f32 * OUTLINE_RATIO).max(1.0);
        let pad = (outline / 2.0 + 1.0).ceil() as i32;

        let mut points = ARROW;
        for point in &mut points {
            point.0 = point.0 * scale + pad as f32;
            point.1 = point.1 * scale + pad as f32;
        }

        let extent = |axis: fn(&(f32, f32)) -> f32| {
            points.iter().map(axis).fold(0.0f32, f32::max).ceil() as i32 + pad
        };
        Self {
            outline,
            pad,
            width: extent(|p| p.0),
            height: extent(|p| p.1),
            points,
        }
    }

    fn rasterise(&self, fill: Color, outline: Color) -> Vec<u8> {
        let mut out = vec![0u8; (self.width * self.height * 4) as usize];
        let half = self.outline / 2.0;

        for y in 0..self.height {
            for x in 0..self.width {
                let d = signed_distance(
                    (x as f32 + 0.5, y as f32 + 0.5),
                    &self.points,
                );
                let covered = coverage(half - d);
                if covered <= 0.0 {
                    continue;
                }
                let inside = coverage(-half - d);
                let color = outline.mix(fill, inside);
                let [r, g, b, a] = color.to_rgba8();
                let a = ((a as f32) * covered).round() as u8;

                let i = ((y * self.width + x) * 4) as usize;
                let m = |c: u8| ((c as u16 * a as u16) / 255) as u8;
                out[i] = m(b);
                out[i + 1] = m(g);
                out[i + 2] = m(r);
                out[i + 3] = a;
            }
        }
        out
    }
}

fn coverage(x: f32) -> f32 {
    (x + 0.5).clamp(0.0, 1.0)
}

fn signed_distance(p: (f32, f32), points: &[(f32, f32)]) -> f32 {
    let mut squared = distance_squared(p, points[0], points[0]);
    let mut sign = 1.0f32;

    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + points.len() - 1) % points.len()];
        squared = squared.min(distance_squared(p, a, b));

        let edge = (b.0 - a.0, b.1 - a.1);
        let to_p = (p.0 - a.0, p.1 - a.1);
        let crossings = [
            p.1 >= a.1,
            p.1 < b.1,
            edge.0 * to_p.1 > edge.1 * to_p.0,
        ];
        if crossings.iter().all(|c| *c) || crossings.iter().all(|c| !*c) {
            sign = -sign;
        }
    }
    sign * squared.sqrt()
}

fn distance_squared(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let edge = (b.0 - a.0, b.1 - a.1);
    let to_p = (p.0 - a.0, p.1 - a.1);
    let length = edge.0 * edge.0 + edge.1 * edge.1;
    let t = if length > 0.0 {
        ((to_p.0 * edge.0 + to_p.1 * edge.1) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let offset = (to_p.0 - edge.0 * t, to_p.1 - edge.1 * t);
    offset.0 * offset.0 + offset.1 * offset.1
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Color = Color::hex(0xffffff);
    const BLACK: Color = Color::hex(0x000000);

    fn alpha(height: i32) -> (Vec<u8>, i32, i32) {
        let art = Art::new(height);
        let pixels = art.rasterise(WHITE, BLACK);
        let alpha = pixels.chunks_exact(4).map(|p| p[3]).collect();
        (alpha, art.width, art.height)
    }

    #[test]
    fn the_arrow_is_as_tall_as_it_was_asked_to_be() {
        for height in [12, 24, 48] {
            let image = CursorImage::new(height, WHITE, BLACK);
            let drawn = image.size.1;
            assert!(
                (drawn - height).abs() <= height / 6 + 3,
                "asked for {height}, drew {drawn}"
            );
        }
    }

    #[test]
    fn the_hotspot_sits_on_the_tip() {
        let art = Art::new(24);
        let pixels = art.rasterise(WHITE, BLACK);
        let (x, y) = (art.pad, art.pad);
        let a = pixels[((y * art.width + x) * 4 + 3) as usize];
        assert!(a > 128, "the pointer must be drawn where it points: alpha {a}");
    }

    #[test]
    fn the_arrow_is_outlined_so_it_shows_on_any_background() {
        let art = Art::new(38);
        let pixels = art.rasterise(WHITE, BLACK);
        let at = |x: i32, y: i32| {
            let i = ((y * art.width + x) * 4) as usize;
            (pixels[i], pixels[i + 3])
        };
        let y = art.height / 3;
        let row: Vec<(u8, u8)> = (0..art.width).map(|x| at(x, y)).collect();
        let solid: Vec<usize> =
            row.iter().enumerate().filter(|(_, (_, a))| *a > 200).map(|(i, _)| i).collect();
        assert!(solid.len() > 4, "the arrow is missing from row {y}");
        let first = solid[0];
        let middle = solid[solid.len() / 2];
        assert!(row[first].0 < 64, "the left edge is the dark colour");
        assert!(row[middle].0 > 192, "and the inside the light one");
    }

    #[test]
    fn most_of_the_buffer_is_transparent() {
        let (alpha, _, _) = alpha(24);
        let opaque = alpha.iter().filter(|a| **a != 0).count();
        assert!(opaque > 0, "an arrow, not an empty buffer");
        assert!(opaque < alpha.len() / 2, "an arrow, not a block");
    }

    #[test]
    fn the_edges_are_feathered_rather_than_jagged() {
        let (alpha, _, _) = alpha(24);
        let partial = alpha.iter().filter(|a| **a > 0 && **a < 255).count();
        assert!(partial > 20, "a hard edge at this size is stairs, not a pointer");
    }

    #[test]
    fn a_bigger_pointer_covers_more() {
        let ink = |height| alpha(height).0.iter().filter(|a| **a > 128).count();
        assert!(ink(48) > ink(24), "the size setting has to do something");
        assert!(ink(24) > ink(12));
    }

    #[test]
    fn an_absurd_size_still_produces_a_pointer() {
        for height in [i32::MIN, -1, 0, 1, 3] {
            let image = CursorImage::new(height, WHITE, BLACK);
            assert!(image.size.0 > 0 && image.size.1 > 0, "a cursor cannot be nothing");
        }
    }

    #[test]
    fn the_inside_of_the_shape_measures_as_inside() {
        let art = Art::new(38);
        let inside = (art.pad as f32 + 4.0, art.pad as f32 + 12.0);
        assert!(signed_distance(inside, &art.points) < 0.0);
        let outside = (art.pad as f32 + 30.0, art.pad as f32 + 2.0);
        assert!(signed_distance(outside, &art.points) > 0.0);
    }

    #[test]
    fn the_distance_to_a_degenerate_edge_is_finite() {
        let p = (1.0, 1.0);
        assert!(distance_squared(p, (0.0, 0.0), (0.0, 0.0)).is_finite());
    }

    #[test]
    #[ignore = "writes a file, only for looking at"]
    fn dump_a_preview() {
        let art = Art::new(24);
        let pixels = art.rasterise(WHITE, BLACK);
        let mut rgba = Vec::with_capacity(pixels.len());
        for px in pixels.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        let path = format!("/tmp/cursor-{}x{}.rgba", art.width, art.height);
        std::fs::write(&path, &rgba).unwrap();
        println!("{path}");
    }
}
