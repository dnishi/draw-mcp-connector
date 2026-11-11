//! Creates the app icon. The same artwork (a tablet and a handwritten line) is used in two places.
//!
//! - The macOS app icon (the image seen in Finder, Launchpad, and the app switcher).
//!   The image files are not committed. The GitHub Actions macOS runner writes PNGs to target/AppIcon.iconset/ with
//!   `cargo test app_icon::tests::write_iconset -- --ignored`,
//!   and iconutil combines them into .icns inside the .app (.github/workflows/bridge.yml).
//! - The canvas page's favicon (`src/ui/public/favicon.svg`) and, for the iPhone / iPad home screen and bookmarks,
//!   `src/ui/public/apple-touch-icon.png`. They are committed because serving the page needs them.
//!   After changing the artwork, regenerate them with `cargo test app_icon::tests::write_web_icons -- --ignored`.
//!
//! The shape is the same "tablet and handwritten line" as the menu bar icon (icon.rs).
//! For macOS, the base uses the macOS app icon shape (an 824 rounded square inside 1024).
//! For the page, the surrounding margin is cut off and the 824 square fills the image (so it stays visible when small).

use crate::geometry::polyline_distance;
use crate::icon::{RoundedRect, handwriting};

/// Drawn in 1024 coordinates
const CANVAS: f32 = 1024.0;
const BACKGROUND: RoundedRect = RoundedRect {
    center: (512.0, 512.0),
    half: (412.0, 412.0),
    radius: 185.0,
};
const BACKGROUND_TOP: [f32; 3] = [0x5b as f32, 0x8c as f32, 0xff as f32];
const BACKGROUND_BOTTOM: [f32; 3] = [0x2f as f32, 0x55 as f32, 0xd4 as f32];
const TABLET: RoundedRect = RoundedRect {
    center: (512.0, 512.0),
    half: (230.0, 320.0),
    radius: 56.0,
};
const TABLET_COLOR: [f32; 3] = [255.0, 255.0, 255.0];
const INK_COLOR: [f32; 3] = [0x1f as f32, 0x2a as f32, 0x44 as f32];
const INK_WIDTH: f32 = 44.0;

/// Which area to draw, and how
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// macOS app icon: draw the whole 1024; transparent outside the rounded base
    MacOs,
    /// For the page (iOS home screen, etc.): draw only the base square filling the image, painted to the corners (iOS rounds them)
    FullBleed,
}

impl Frame {
    /// Top left of the drawn area (same for x and y) and its side length (1024 coordinates)
    fn area(self) -> (f32, f32) {
        match self {
            Self::MacOs => (0.0, CANVAS),
            Self::FullBleed => {
                let left = BACKGROUND.center.0 - BACKGROUND.half.0;
                (left, BACKGROUND.half.0 * 2.0)
            }
        }
    }
}

fn ink() -> Vec<(f32, f32)> {
    handwriting(360.0, 640.0, 304.0, 190.0, 64.0)
}

/// From the distance inward from the edge (pixels), the fraction of the pixel covered by the shape (smooths the edges)
fn coverage(signed_distance_px: f32) -> f32 {
    (0.5 - signed_distance_px).clamp(0.0, 1.0)
}

/// size x size RGBA
pub fn rgba(size: u32, frame: Frame) -> Vec<u8> {
    let (origin, extent) = frame.area();
    let scale = size as f32 / extent;
    let ink = ink();
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let p = (
                origin + (px as f32 + 0.5) / scale,
                origin + (py as f32 + 0.5) / scale,
            );
            let t = p.1 / CANVAS;
            let background: [f32; 3] =
                std::array::from_fn(|i| BACKGROUND_TOP[i] * (1.0 - t) + BACKGROUND_BOTTOM[i] * t);

            // Layer from the bottom up (colors 0 to 255, alpha 0 to 1)
            let mut color = background;
            let mut alpha = match frame {
                Frame::MacOs => coverage(BACKGROUND.distance(p) * scale),
                Frame::FullBleed => 1.0,
            };
            let layers = [
                (TABLET_COLOR, coverage(TABLET.distance(p) * scale)),
                (
                    INK_COLOR,
                    coverage((polyline_distance(p, &ink) - INK_WIDTH / 2.0) * scale),
                ),
            ];
            for (layer, a) in layers {
                color = std::array::from_fn(|i| layer[i] * a + color[i] * (1.0 - a));
            }
            // Transparent outside the background (outside the corners)
            if alpha == 0.0 {
                color = [0.0; 3];
            }
            alpha = alpha.clamp(0.0, 1.0);
            pixels.extend(color.map(|c| c.round() as u8));
            pixels.push((alpha * 255.0).round() as u8);
        }
    }
    pixels
}

fn hex(color: [f32; 3]) -> String {
    let [r, g, b] = color.map(|c| c as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn svg_rect(rect: &RoundedRect, fill: &str) -> String {
    format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{fill}"/>"#,
        rect.center.0 - rect.half.0,
        rect.center.1 - rect.half.1,
        rect.half.0 * 2.0,
        rect.half.1 * 2.0,
        rect.radius,
    )
}

/// The page's favicon (SVG). Same shape and colors as rgba; the area matches Frame::FullBleed (corners stay rounded)
pub fn favicon_svg() -> String {
    let (origin, extent) = Frame::FullBleed.area();
    let points: Vec<String> = ink()
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect();
    let lines = [
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{origin} {origin} {extent} {extent}">"#
        ),
        // Like rgba, the base color changes from the top to the bottom of the 1024 area
        format!(
            r#"<defs><linearGradient id="background" x1="0" y1="0" x2="0" y2="{CANVAS}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient></defs>"#,
            hex(BACKGROUND_TOP),
            hex(BACKGROUND_BOTTOM),
        ),
        svg_rect(&BACKGROUND, "url(#background)"),
        svg_rect(&TABLET, &hex(TABLET_COLOR)),
        format!(
            r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="{INK_WIDTH}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            points.join(" "),
            hex(INK_COLOR),
        ),
        "</svg>".to_string(),
    ];
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File names and sizes iconutil expects (the .iconset layout described for Apple's iconutil)
    const ICONSET: [(&str, u32); 10] = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ];

    /// Size of the iOS home screen icon: 180px, the largest in Apple's example (Safari Web Content Guide), for iPhone Retina;
    /// iPad (152 / 167px) also uses this single image
    const APPLE_TOUCH_ICON_SIZE: u32 = 180;
    const WEB_ICONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/public");

    /// PNG (RGBA, 8 bits)
    fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        png
    }

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> &[u8] {
        &pixels[((y * size + x) * 4) as usize..][..4]
    }

    #[test]
    fn mac_os_icon_has_transparent_corners_and_white_tablet() {
        let size = 64;
        let pixels = rgba(size, Frame::MacOs);
        assert_eq!(pixel(&pixels, size, 0, 0)[3], 0, "角の外は透明");
        // At 1024 coordinates, y=307 is inside the tablet (y=192 to 832) and above the handwritten line
        assert_eq!(
            pixel(&pixels, size, size / 2, size * 3 / 10),
            [255, 255, 255, 255],
            "タブレットは白"
        );
    }

    #[test]
    fn full_bleed_icon_paints_corners() {
        let size = 64;
        let pixels = rgba(size, Frame::FullBleed);
        assert_eq!(pixel(&pixels, size, 0, 0)[3], 255, "角まで塗る");
        assert_eq!(pixel(&pixels, size, size - 1, size - 1)[3], 255);
    }

    #[test]
    fn favicon_uses_same_shapes_and_colors() {
        let svg = favicon_svg();
        assert!(
            svg.starts_with(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="100 100 824 824">"#
            )
        );
        assert!(svg.contains(
            r##"<rect x="282" y="192" width="460" height="640" rx="56" fill="#ffffff"/>"##
        ));
        assert!(svg.contains(r##"stop-color="#5b8cff""##));
        assert!(svg.contains(r##"stroke="#1f2a44""##));
    }

    #[test]
    #[ignore]
    fn write_iconset() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/AppIcon.iconset");
        std::fs::create_dir_all(dir).unwrap();
        for (name, size) in ICONSET {
            std::fs::write(
                format!("{dir}/{name}"),
                png(size, &rgba(size, Frame::MacOs)),
            )
            .unwrap();
        }
    }

    /// Writes the canvas page icons to src/ui/public/
    #[test]
    #[ignore]
    fn write_web_icons() {
        std::fs::write(format!("{WEB_ICONS_DIR}/favicon.svg"), favicon_svg()).unwrap();
        let size = APPLE_TOUCH_ICON_SIZE;
        std::fs::write(
            format!("{WEB_ICONS_DIR}/apple-touch-icon.png"),
            png(size, &rgba(size, Frame::FullBleed)),
        )
        .unwrap();
    }
}
