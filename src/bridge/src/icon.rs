//! Menu bar icon. A tablet frame and a handwritten line show it is a handwriting app,
//! and the color of the dot at the bottom right shows the connection state with the canvas (no dot when disconnected).
//!
//! It is shown as a colored image (a template image can only use one color),
//! so the frame and line are drawn in colors matching the OS appearance (light / dark).

use crate::geometry::polyline_distance;

/// Width and height in pixels. tray-icon sizes icons to 22pt high in the macOS menu bar
/// (MAX_ICON_HEIGHT in tray-icon-0.26.1 `src/platform_impl/macos/mod.rs`). Doubled for Retina
pub const SIZE: u32 = 44;
/// Drawn in 22pt coordinates and scaled to SIZE
const SCALE: f32 = SIZE as f32 / 22.0;
/// How many samples per pixel in each direction to smooth the edges
const SAMPLES: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// Connection state shown by the icon
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    None,
    Waiting,
    Connected,
}

#[derive(Clone, Copy)]
struct Rgb(f32, f32, f32);

impl Rgb {
    const fn hex(value: u32) -> Self {
        Self(
            ((value >> 16) & 0xff) as f32 / 255.0,
            ((value >> 8) & 0xff) as f32 / 255.0,
            (value & 0xff) as f32 / 255.0,
        )
    }
}

/// Color of the frame and line (like menu bar text: black in light, white in dark)
fn foreground(appearance: Appearance) -> Rgb {
    match appearance {
        Appearance::Light => Rgb::hex(0x000000),
        Appearance::Dark => Rgb::hex(0xffffff),
    }
}

/// Dot color. Apple system colors (Human Interface Guidelines, Color) for light / dark
fn badge_color(badge: Badge, appearance: Appearance) -> Option<Rgb> {
    let (light, dark) = match badge {
        Badge::None => return None,
        Badge::Waiting => (0xff9500, 0xff9f0a),   // Orange
        Badge::Connected => (0x34c759, 0x30d158), // Green
    };
    Some(Rgb::hex(match appearance {
        Appearance::Light => light,
        Appearance::Dark => dark,
    }))
}

// ---- Shapes (22pt coordinates) ----

/// Rounded rectangle
pub(crate) struct RoundedRect {
    pub center: (f32, f32),
    pub half: (f32, f32),
    pub radius: f32,
}

impl RoundedRect {
    /// Signed distance to the edge (negative inside)
    pub fn distance(&self, (x, y): (f32, f32)) -> f32 {
        let dx = (x - self.center.0).abs() - (self.half.0 - self.radius);
        let dy = (y - self.center.1).abs() - (self.half.1 - self.radius);
        let outside = dx.max(0.0).hypot(dy.max(0.0));
        outside + dx.max(dy).min(0.0) - self.radius
    }
}

/// Handwritten line: a wave of amplitude `amplitude` rising by `rise` to the right over `width` from (left, base)
pub(crate) fn handwriting(
    left: f32,
    base: f32,
    width: f32,
    rise: f32,
    amplitude: f32,
) -> Vec<(f32, f32)> {
    const STEPS: usize = 48;
    (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            let wave = (t * std::f32::consts::TAU * 1.5).sin();
            (left + width * t, base - rise * t + amplitude * wave)
        })
        .collect()
}

const TABLET: RoundedRect = RoundedRect {
    center: (9.5, 11.0),
    half: (6.0, 9.0),
    radius: 2.2,
};
const TABLET_STROKE: f32 = 1.6;
const PEN_STROKE: f32 = 1.5;
const BADGE_CENTER: (f32, f32) = (17.2, 16.6);
const BADGE_RADIUS: f32 = 3.4;
/// Outline of the dot (same color as the frame), so its shape stays visible when the menu bar color is close to the dot color
const BADGE_EDGE: f32 = 1.0;
/// Area around the dot where the frame and line are cleared (so the dot does not look attached to the frame)
const BADGE_GAP: f32 = 1.2;

fn pen_points() -> Vec<(f32, f32)> {
    handwriting(6.0, 14.5, 7.0, 4.0, 1.6)
}

/// Color of one point (RGB premultiplied by opacity, and opacity)
fn shade(p: (f32, f32), pen: &[(f32, f32)], fg: Rgb, badge: Option<Rgb>) -> (f32, f32, f32, f32) {
    let badge_distance = (p.0 - BADGE_CENTER.0).hypot(p.1 - BADGE_CENTER.1);
    if let Some(color) = badge {
        if badge_distance <= BADGE_RADIUS {
            return (color.0, color.1, color.2, 1.0);
        }
        if badge_distance <= BADGE_RADIUS + BADGE_EDGE {
            return (fg.0, fg.1, fg.2, 1.0);
        }
        if badge_distance <= BADGE_RADIUS + BADGE_EDGE + BADGE_GAP {
            return (0.0, 0.0, 0.0, 0.0);
        }
    }
    let on_tablet = TABLET.distance(p).abs() <= TABLET_STROKE / 2.0;
    let on_pen = polyline_distance(p, pen) <= PEN_STROKE / 2.0;
    if on_tablet || on_pen {
        (fg.0, fg.1, fg.2, 1.0)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    }
}

/// SIZE x SIZE RGBA (row by row, 4 bytes per pixel)
pub fn rgba(badge: Badge, appearance: Appearance) -> Vec<u8> {
    let pen = pen_points();
    let fg = foreground(appearance);
    let badge = badge_color(badge, appearance);
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for py in 0..SIZE {
        for px in 0..SIZE {
            let mut sum = (0.0, 0.0, 0.0, 0.0);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / SCALE;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / SCALE;
                    let (r, g, b, a) = shade((x, y), &pen, fg, badge);
                    sum = (sum.0 + r * a, sum.1 + g * a, sum.2 + b * a, sum.3 + a);
                }
            }
            let alpha = sum.3 / (SAMPLES * SAMPLES) as f32;
            let channel = |premultiplied: f32| {
                if sum.3 == 0.0 {
                    0
                } else {
                    (premultiplied / sum.3 * 255.0).round() as u8
                }
            };
            pixels.extend([
                channel(sum.0),
                channel(sum.1),
                channel(sum.2),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], x: f32, y: f32) -> [u8; 4] {
        let (px, py) = ((x * SCALE) as usize, (y * SCALE) as usize);
        let i = (py * SIZE as usize + px) * 4;
        pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn foreground_follows_appearance() {
        // Top of the frame's left edge
        let (x, y) = (TABLET.center.0 - TABLET.half.0, TABLET.center.1);
        assert_eq!(
            pixel(&rgba(Badge::None, Appearance::Light), x, y),
            [0, 0, 0, 255]
        );
        assert_eq!(
            pixel(&rgba(Badge::None, Appearance::Dark), x, y),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn badge_shows_status_color() {
        let (x, y) = BADGE_CENTER;
        assert_eq!(
            pixel(&rgba(Badge::None, Appearance::Light), x, y)[3],
            0,
            "未接続は丸なし"
        );
        assert_eq!(
            pixel(&rgba(Badge::Connected, Appearance::Light), x, y),
            [0x34, 0xc7, 0x59, 255]
        );
        assert_eq!(
            pixel(&rgba(Badge::Waiting, Appearance::Dark), x, y),
            [0xff, 0x9f, 0x0a, 255]
        );
    }

    #[test]
    fn background_is_transparent() {
        assert_eq!(
            pixel(&rgba(Badge::Connected, Appearance::Light), 0.2, 0.2)[3],
            0
        );
    }

    /// For visual checks. `cargo test icon::tests::write_preview -- --ignored`
    /// writes RGBA to target/icon-preview/
    #[test]
    #[ignore]
    fn write_preview() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/icon-preview");
        std::fs::create_dir_all(dir).unwrap();
        for (badge, b) in [
            (Badge::None, "none"),
            (Badge::Waiting, "waiting"),
            (Badge::Connected, "connected"),
        ] {
            for (appearance, a) in [(Appearance::Light, "light"), (Appearance::Dark, "dark")] {
                std::fs::write(format!("{dir}/{b}-{a}.rgba"), rgba(badge, appearance)).unwrap();
            }
        }
    }
}
