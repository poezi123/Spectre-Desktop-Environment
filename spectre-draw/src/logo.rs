//! The Spectre mark.
//!
//! The hexagonal S from `assets/Logoofficial.png`, stored as raw premultiplied
//! RGBA rather than as a PNG: decoding a compressed image would mean pulling a
//! decoder into every process that wants to show the logo, and a 64x64 master
//! is 16 KiB of read-only data that never has to be decompressed at all.
//!
//! The blob is generated from the source render by cropping away the wordmark,
//! trimming to the mark's bounding box, padding to a square, resampling to
//! 64x64 and multiplying the colour channels by alpha.

use spectre_text::Image;

/// Edge length of the baked master, in pixels.
pub const MASTER: u32 = 64;

/// 64x64 premultiplied RGBA, row major.
static PIXELS: &[u8] = include_bytes!("../assets/logo-64.rgba");

/// The Spectre mark at `size` x `size`, ready for [`Canvas::draw_image`].
///
/// Sizes below the master are box filtered, which is what keeps the thin
/// contour lines inside the mark from breaking up at panel sizes; above it the
/// master is sampled directly and the result is soft, which is the honest
/// outcome of asking for more detail than the blob has.
///
/// [`Canvas::draw_image`]: crate::Canvas::draw_image
pub fn logo(size: u32) -> Image {
    if size == 0 {
        return Image::empty();
    }

    let mut data = vec![0u8; size as usize * size as usize * 4];
    let step = MASTER as f32 / size as f32;
    let gain = gain(size);

    for y in 0..size {
        let y0 = y as f32 * step;
        let y1 = y0 + step;
        for x in 0..size {
            let x0 = x as f32 * step;
            let x1 = x0 + step;

            let mut sum = [0f32; 4];
            let mut weight = 0.0;
            for sy in y0.floor() as u32..(y1.ceil() as u32).min(MASTER) {
                let cover_y = coverage(sy, y0, y1);
                for sx in x0.floor() as u32..(x1.ceil() as u32).min(MASTER) {
                    let w = cover_y * coverage(sx, x0, x1);
                    if w <= 0.0 {
                        continue;
                    }
                    let i = (sy as usize * MASTER as usize + sx as usize) * 4;
                    for c in 0..4 {
                        sum[c] += PIXELS[i + c] as f32 * w;
                    }
                    weight += w;
                }
            }

            let out = (y as usize * size as usize + x as usize) * 4;
            if weight > 0.0 {
                let alpha = sum[3] / weight;
                for c in 0..3 {
                    // Clamped to alpha: the canvas blends premultiplied pixels,
                    // and a channel above its own alpha would fringe.
                    data[out + c] = (sum[c] / weight * gain).min(alpha).round().clamp(0.0, 255.0) as u8;
                }
                data[out + 3] = alpha.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    Image { width: size, height: size, data }
}

/// Brightness lift applied to small renderings.
///
/// Box filtering the master down to panel size averages the bright channel
/// running through the mark into the near-black body around it, and on a
/// near-black panel the result goes muddy. The gain gives back the contrast
/// the downscale spends; at the master's own size it is a no-op.
fn gain(size: u32) -> f32 {
    const FULL: f32 = 44.0;
    if size as f32 >= FULL {
        return 1.0;
    }
    1.0 + (FULL - size as f32) / FULL * 0.9
}

/// How much of source pixel `i` falls inside `[start, end)`.
fn coverage(i: u32, start: f32, end: f32) -> f32 {
    let lo = (i as f32).max(start);
    let hi = ((i + 1) as f32).min(end);
    (hi - lo).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_master_blob_is_the_size_it_claims() {
        assert_eq!(PIXELS.len(), (MASTER * MASTER * 4) as usize);
    }

    #[test]
    fn a_zero_sized_logo_is_empty_rather_than_a_panic() {
        assert!(logo(0).is_empty());
    }

    #[test]
    fn the_logo_comes_out_at_the_requested_size() {
        for size in [16, 22, 24, 44, 64, 96] {
            let image = logo(size);
            assert_eq!(image.width, size);
            assert_eq!(image.height, size);
            assert_eq!(image.data.len(), image.stride() * size as usize);
        }
    }

    #[test]
    fn the_mark_is_still_visible_at_panel_size() {
        let image = logo(22);
        let opaque = image.data.chunks(4).filter(|p| p[3] > 32).count();
        assert!(opaque > 100, "only {opaque} solid pixels left of the mark");
    }

    #[test]
    fn nothing_comes_out_unpremultiplied() {
        // A colour channel above alpha would show up as a bright fringe once
        // the canvas blends it.
        let image = logo(24);
        for p in image.data.chunks(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "{p:?}");
        }
    }

    #[test]
    fn small_renderings_are_lifted_and_large_ones_are_left_alone() {
        assert_eq!(gain(64), 1.0);
        assert_eq!(gain(44), 1.0);
        assert!(gain(22) > gain(44));
        assert!(gain(16) > gain(22));
    }

    #[test]
    fn the_panel_sized_mark_is_brighter_than_the_untouched_master() {
        let brightness = |image: &Image| {
            let lit: Vec<_> = image.data.chunks(4).filter(|p| p[3] > 128).collect();
            lit.iter().map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32).sum::<u32>()
                / lit.len().max(1) as u32
        };
        assert!(brightness(&logo(22)) > brightness(&logo(64)));
    }

    #[test]
    fn the_corners_stay_transparent() {
        let image = logo(32);
        let at = |x: usize, y: usize| image.data[(y * 32 + x) * 4 + 3];
        assert_eq!(at(0, 0), 0);
        assert_eq!(at(31, 0), 0);
        assert_eq!(at(0, 31), 0);
        assert_eq!(at(31, 31), 0);
    }
}
