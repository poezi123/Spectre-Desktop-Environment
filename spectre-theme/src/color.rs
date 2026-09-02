//! Colour primitives.
//!
//! Everything is stored as non-premultiplied sRGB with a linear alpha, which is
//! what both Cairo and the GLES renderer expect. Conversion to linear light only
//! happens inside a shader, never here.

use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight alpha, each channel in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    /// Build a colour from `0xRRGGBB`. Intended for the palette constants below,
    /// where the literal reads much better than four floats.
    pub const fn hex(v: u32) -> Self {
        Self::hex_a(v, 255)
    }

    pub const fn hex_a(v: u32, alpha: u8) -> Self {
        Self::rgba(
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
            alpha as f32 / 255.0,
        )
    }

    /// Same colour at a different opacity.
    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Scale the RGB channels, keeping alpha. `f > 1.0` brightens.
    pub fn scaled(self, f: f32) -> Self {
        Self {
            r: (self.r * f).clamp(0.0, 1.0),
            g: (self.g * f).clamp(0.0, 1.0),
            b: (self.b * f).clamp(0.0, 1.0),
            a: self.a,
        }
    }

    /// Linear interpolation in sRGB space. Good enough for the short hops the
    /// accent gradient makes; a full Oklab mix is not worth the cost here.
    pub fn mix(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// Straight alpha over an opaque backdrop.
    pub fn over(self, backdrop: Color) -> Self {
        backdrop.mix(self.alpha(1.0), self.a)
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Premultiplied, which is what the GLES renderer wants for blending.
    pub fn to_premultiplied(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [c(self.r), c(self.g), c(self.b), c(self.a)]
    }

    fn parse_hex(s: &str) -> Result<Self, ParseColorError> {
        let h = s.strip_prefix('#').unwrap_or(s);
        let n = |from: usize, to: usize| -> Result<u8, ParseColorError> {
            u8::from_str_radix(&h[from..to], 16).map_err(|_| ParseColorError(s.to_owned()))
        };
        // Accept #rgb, #rgba, #rrggbb and #rrggbbaa.
        let expand = |v: u8| v * 17;
        match h.len() {
            3 | 4 => {
                let mut c = [255u8; 4];
                for (i, slot) in c.iter_mut().take(h.len()).enumerate() {
                    *slot = expand(n(i, i + 1)?);
                }
                Ok(Self::rgba(
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                    c[3] as f32 / 255.0,
                ))
            }
            6 | 8 => {
                let mut c = [255u8; 4];
                for (i, slot) in c.iter_mut().take(h.len() / 2).enumerate() {
                    *slot = n(i * 2, i * 2 + 2)?;
                }
                Ok(Self::rgba(
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                    c[3] as f32 / 255.0,
                ))
            }
            _ => Err(ParseColorError(s.to_owned())),
        }
    }
}

impl std::str::FromStr for Color {
    type Err = ParseColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_hex(s.trim())
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else {
            write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

/// The string was not a `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColorError(String);

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a hex colour", self.0)
    }
}

impl std::error::Error for ParseColorError {}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(de::Error::custom)
    }
}

/// A multi-stop gradient, used for the RGB accent that runs along focused
/// window borders, the active workspace pip and the panel underline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub stops: Vec<Color>,
}

impl Gradient {
    pub fn new(stops: impl Into<Vec<Color>>) -> Self {
        Self { stops: stops.into() }
    }

    /// Flat gradient from a single colour, for `accent = "#rrggbb"` configs.
    pub fn solid(color: Color) -> Self {
        Self { stops: vec![color] }
    }

    /// Sample at `t` in `0.0..=1.0`. An empty gradient samples transparent so a
    /// broken config degrades to "no accent" instead of panicking mid-frame.
    pub fn sample(&self, t: f32) -> Color {
        match self.stops.len() {
            0 => Color::TRANSPARENT,
            1 => self.stops[0],
            n => {
                let t = t.clamp(0.0, 1.0) * (n - 1) as f32;
                let i = (t.floor() as usize).min(n - 2);
                self.stops[i].mix(self.stops[i + 1], t - i as f32)
            }
        }
    }

    /// Sample at `t` with the stops treated as a loop, so a cycle through the
    /// gradient has no seam.
    pub fn sample_cyclic(&self, t: f32) -> Color {
        match self.stops.len() {
            0 => Color::TRANSPARENT,
            1 => self.stops[0],
            n => {
                let t = t.rem_euclid(1.0) * n as f32;
                let i = (t.floor() as usize) % n;
                self.stops[i].mix(self.stops[(i + 1) % n], t - t.floor())
            }
        }
    }

    /// Average colour, for places that need one flat value (a 1px border on a
    /// low-end profile, an icon tint).
    pub fn average(&self) -> Color {
        if self.stops.is_empty() {
            return Color::TRANSPARENT;
        }
        let n = self.stops.len() as f32;
        let sum = self.stops.iter().fold([0.0f32; 4], |mut acc, c| {
            acc[0] += c.r;
            acc[1] += c.g;
            acc[2] += c.b;
            acc[3] += c.a;
            acc
        });
        Color::rgba(sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n)
    }

    pub fn scaled(&self, f: f32) -> Self {
        Self::new(self.stops.iter().map(|c| c.scaled(f)).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_hex_length() {
        assert_eq!("#fff".parse::<Color>().unwrap(), Color::hex(0xffffff));
        assert_eq!("0a0b0d".parse::<Color>().unwrap(), Color::hex(0x0a0b0d));
        assert_eq!("#0a0b0d80".parse::<Color>().unwrap(), Color::hex_a(0x0a0b0d, 0x80));
        assert_eq!("#f008".parse::<Color>().unwrap(), Color::hex_a(0xff0000, 0x88));
    }

    #[test]
    fn rejects_junk() {
        assert!("#gg0000".parse::<Color>().is_err());
        assert!("#12345".parse::<Color>().is_err());
        assert!("".parse::<Color>().is_err());
    }

    #[test]
    fn display_round_trips() {
        let c = Color::hex(0x16a3c8);
        assert_eq!(c.to_string(), "#16a3c8");
        assert_eq!(c.to_string().parse::<Color>().unwrap(), c);
        assert_eq!(Color::hex_a(0x16a3c8, 0x40).to_string(), "#16a3c840");
    }

    #[test]
    fn gradient_samples_endpoints_and_middle() {
        let g = Gradient::new(vec![Color::hex(0x000000), Color::hex(0xffffff)]);
        assert_eq!(g.sample(0.0), Color::hex(0x000000));
        assert_eq!(g.sample(1.0), Color::hex(0xffffff));
        assert_eq!(g.sample(0.5).to_rgba8(), [128, 128, 128, 255]);
    }

    #[test]
    fn empty_gradient_is_transparent_not_a_panic() {
        assert_eq!(Gradient::new(vec![]).sample(0.5), Color::TRANSPARENT);
        assert_eq!(Gradient::new(vec![]).average(), Color::TRANSPARENT);
    }

    #[test]
    fn premultiply_matches_alpha() {
        let c = Color::hex(0xffffff).alpha(0.5);
        assert_eq!(c.to_premultiplied(), [0.5, 0.5, 0.5, 0.5]);
    }
}
