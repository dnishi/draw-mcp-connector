//! Draws the canvas as PNG, so AI agents see strokes as an image instead of lists of coordinates.
//!
//! Line width and corner rounding match the canvas page's drawing (`radiusAt` and `smoothPoints` in
//! `src/ui/src/features/canvas/stroke.ts`). Like the page, it draws on white in each stroke's color.

use serde::Serialize;

use std::ops::Range;

use crate::color;
use crate::geometry::{Point, closest_on_segment};
use crate::protocol::{PageSize, Stroke, StrokePoint, Viewport};

/// Line radius at pressure 1 (CSS pixels). PEN_MAX_RADIUS in stroke.ts
const PEN_MAX_RADIUS: f64 = 3.0;
/// Lower bound so lines do not vanish when pressure is near 0. MIN_PRESSURE in stroke.ts
const MIN_PRESSURE: f64 = 0.25;
/// Upper bound of the image's long side (pixels). Larger areas are drawn scaled down
const MAX_IMAGE_SIDE: f64 = 2000.0;
/// Lower bound of the radius on the image (pixels), so lines do not vanish when scaled down
const MIN_PIXEL_RADIUS: f32 = 0.5;

fn radius_at(pressure: f64) -> f64 {
    PEN_MAX_RADIUS * pressure.max(MIN_PRESSURE)
}

/// A rectangle on the canvas (CSS pixels)
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn from_edges(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    fn right(&self) -> f64 {
        self.x + self.width
    }

    fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn union(self, other: Self) -> Self {
        Self::from_edges(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }
}

impl From<Viewport> for Rect {
    fn from(v: Viewport) -> Self {
        Self {
            x: v.x,
            y: v.y,
            width: v.width,
            height: v.height,
        }
    }
}

impl From<PageSize> for Rect {
    /// The paper starts at (0, 0)
    fn from(page: PageSize) -> Self {
        Self::from_edges(0.0, 0.0, page.width, page.height)
    }
}

/// Area covered by a stroke's ink (including line width). None if there are no points
pub fn stroke_bounds(stroke: &Stroke) -> Option<Rect> {
    stroke
        .points
        .iter()
        .map(|p| {
            let r = radius_at(p.pressure);
            Rect::from_edges(p.x - r, p.y - r, p.x + r, p.y + r)
        })
        .reduce(Rect::union)
}

fn lerp(a: &StrokePoint, b: &StrokePoint, t: f64) -> StrokePoint {
    StrokePoint {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        pressure: a.pressure + (b.pressure - a.pressure) * t,
    }
}

/// Rounds corners once with Chaikin's algorithm. The first and last points do not move (smoothPoints in stroke.ts)
fn smooth_points(points: &[StrokePoint]) -> Vec<StrokePoint> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut smoothed = vec![first.clone()];
    for pair in points.windows(2) {
        smoothed.push(lerp(&pair[0], &pair[1], 0.25));
        smoothed.push(lerp(&pair[0], &pair[1], 0.75));
    }
    smoothed.push(last.clone());
    smoothed
}

/// The drawn image and which part of the canvas it shows
pub struct CanvasImage {
    pub png: Vec<u8>,
    /// Area of the canvas shown in the image. The image's top left is this area's top left
    pub area: Rect,
    /// Image pixels per canvas CSS pixel (1 or less; large areas are scaled down)
    pub scale: f64,
}

/// Draws `regions` (areas that must be shown, such as the paper and visible areas) and the area containing every stroke. None if neither exists
pub fn render(
    strokes: &[Stroke],
    regions: &[Rect],
) -> Result<Option<CanvasImage>, png::EncodingError> {
    let Some(area) = regions
        .iter()
        .copied()
        .chain(strokes.iter().filter_map(stroke_bounds))
        .reduce(Rect::union)
    else {
        return Ok(None);
    };
    // Align the edges to CSS pixel boundaries
    let area = Rect::from_edges(
        area.x.floor(),
        area.y.floor(),
        area.right().ceil(),
        area.bottom().ceil(),
    );
    // If both width and height are 0, MAX_IMAGE_SIDE / 0 is infinite and clamps to 1
    let scale = (MAX_IMAGE_SIDE / area.width.max(area.height)).min(1.0);
    // Use round, because ceil adds a pixel from rounding errors when scaling down (2000.0000001, etc.)
    let pixels = |length: f64| ((length * scale).round() as u32).max(1);

    let mut raster = Raster::new(pixels(area.width), pixels(area.height));
    for stroke in strokes {
        raster.draw_stroke(stroke, area, scale);
    }
    Ok(Some(CanvasImage {
        png: raster.to_png()?,
        area,
        scale,
    }))
}

/// A point on the image and the line radius there (pixels)
type PixelPoint = (Point, f32);

/// Pixel range (right and bottom exclusive). Empty if nothing is inside
struct PixelBounds {
    xs: Range<u32>,
    ys: Range<u32>,
}

impl PixelBounds {
    const EMPTY: Self = Self { xs: 0..0, ys: 0..0 };

    fn include(&mut self, xs: &Range<u32>, ys: &Range<u32>) {
        if xs.is_empty() || ys.is_empty() {
            return;
        }
        if self.xs.is_empty() {
            (self.xs, self.ys) = (xs.clone(), ys.clone());
            return;
        }
        self.xs = self.xs.start.min(xs.start)..self.xs.end.max(xs.end);
        self.ys = self.ys.start.min(ys.start)..self.ys.end.max(ys.end);
    }
}

/// Paints strokes one by one, each in its color, over white
struct Raster {
    width: u32,
    height: u32,
    /// Pixel colors (RGB 0 to 1)
    rgb: Vec<[f32; 3]>,
    /// Ink density per pixel (0 to 1) of the stroke being painted.
    /// Takes the maximum before mixing into the color, so overlapping segments of the same stroke do not get darker
    coverage: Vec<f32>,
    /// Range of pixels touched by the stroke being painted (only this range is mixed into colors and reset to 0)
    touched: PixelBounds,
}

impl Raster {
    fn new(width: u32, height: u32) -> Self {
        let pixels = width as usize * height as usize;
        Self {
            width,
            height,
            rgb: vec![[1.0; 3]; pixels],
            coverage: vec![0.0; pixels],
            touched: PixelBounds::EMPTY,
        }
    }

    fn draw_stroke(&mut self, stroke: &Stroke, area: Rect, scale: f64) {
        let to_pixel = |p: &StrokePoint| -> PixelPoint {
            let x = ((p.x - area.x) * scale) as f32;
            let y = ((p.y - area.y) * scale) as f32;
            let radius = ((radius_at(p.pressure) * scale) as f32).max(MIN_PIXEL_RADIUS);
            ((x, y), radius)
        };
        let points: Vec<PixelPoint> = smooth_points(&stroke.points).iter().map(to_pixel).collect();
        match points.as_slice() {
            [] => {}
            [only] => self.cover_segment(*only, *only),
            _ => {
                for pair in points.windows(2) {
                    self.cover_segment(pair[0], pair[1]);
                }
            }
        }
        // Draw malformed colors (bad data) in black
        let color = color::parse(&stroke.color).unwrap_or([0, 0, 0]);
        self.blend(color.map(|c| f32::from(c) / 255.0));
    }

    /// Writes into coverage the density of a round-capped segment whose radius changes from a's to b's
    fn cover_segment(&mut self, (a, radius_a): PixelPoint, (b, radius_b): PixelPoint) {
        let reach = radius_a.max(radius_b) + 1.0;
        let xs = pixel_range(a.0.min(b.0) - reach, a.0.max(b.0) + reach, self.width);
        let ys = pixel_range(a.1.min(b.1) - reach, a.1.max(b.1) + reach, self.height);
        self.touched.include(&xs, &ys);
        for py in ys {
            for px in xs.clone() {
                let center = (px as f32 + 0.5, py as f32 + 0.5);
                let (t, distance) = closest_on_segment(center, a, b);
                let radius = radius_a + (radius_b - radius_a) * t;
                // Make the 1-pixel edge an intermediate density for smoothing
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                let cell = &mut self.coverage[py as usize * self.width as usize + px as usize];
                *cell = cell.max(coverage);
            }
        }
    }

    /// Mixes color in by coverage density and resets coverage to 0
    fn blend(&mut self, color: [f32; 3]) {
        let touched = std::mem::replace(&mut self.touched, PixelBounds::EMPTY);
        for py in touched.ys {
            for px in touched.xs.clone() {
                let i = py as usize * self.width as usize + px as usize;
                let coverage = std::mem::take(&mut self.coverage[i]);
                let pixel = &mut self.rgb[i];
                for (channel, ink) in pixel.iter_mut().zip(color) {
                    *channel += (ink - *channel) * coverage;
                }
            }
        }
    }

    fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let bytes: Vec<u8> = self
            .rgb
            .iter()
            .flat_map(|pixel| pixel.map(|channel| (channel * 255.0).round() as u8))
            .collect();
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&bytes)?;
        writer.finish()?;
        Ok(png)
    }
}

/// Indices of pixels covering start to end (pixel coordinates). Excludes pixels outside the image
fn pixel_range(start: f32, end: f32, limit: u32) -> Range<u32> {
    let end = (end.ceil().max(0.0) as u32).min(limit);
    let start = (start.floor().max(0.0) as u32).min(end);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Author;

    fn stroke(points: &[(f64, f64)]) -> Stroke {
        Stroke {
            id: "s".into(),
            author: Author::Human,
            color: crate::color::BLACK.into(),
            points: points
                .iter()
                .map(|&(x, y)| StrokePoint {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
        }
    }

    /// Reads the PNG back into (width, height, RGB pixels)
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        (info.width, info.height, pixels)
    }

    #[test]
    fn bounds_include_line_width() {
        let bounds = stroke_bounds(&stroke(&[(10.0, 20.0), (30.0, 25.0)])).unwrap();
        assert_eq!(bounds, Rect::from_edges(7.0, 17.0, 33.0, 28.0));
        assert_eq!(stroke_bounds(&stroke(&[])), None);
    }

    #[test]
    fn nothing_to_draw_without_viewport_or_strokes() {
        assert!(render(&[], &[]).unwrap().is_none());
    }

    #[test]
    fn draws_black_line_on_white_viewport() {
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0,
        };
        let image = render(&[stroke(&[(5.0, 10.0), (35.0, 10.0)])], &[viewport.into()])
            .unwrap()
            .unwrap();
        assert_eq!(image.area, Rect::from_edges(0.0, 0.0, 40.0, 20.0));
        assert_eq!(image.scale, 1.0);

        let (width, height, pixels) = decode(&image.png);
        assert_eq!((width, height), (40, 20));
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(20, 10), [0, 0, 0], "線の上は黒");
        assert_eq!(at(20, 2), [255, 255, 255], "線から離れた所は白");
    }

    #[test]
    fn draws_each_stroke_in_its_color() {
        let red = Stroke {
            color: "#ff3b30".into(),
            ..stroke(&[(5.0, 10.0), (35.0, 10.0)])
        };
        // Draw a crossing red line over a black line
        let black = stroke(&[(20.0, 0.0), (20.0, 20.0)]);
        let image = render(&[black, red], &[Rect::from_edges(0.0, 0.0, 40.0, 20.0)])
            .unwrap()
            .unwrap();

        let (width, _, pixels) = decode(&image.png);
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(10, 10), [0xff, 0x3b, 0x30], "赤の線");
        assert_eq!(at(20, 10), [0xff, 0x3b, 0x30], "後から書いた赤が上に重なる");
        assert_eq!(at(20, 3), [0, 0, 0], "黒の線");
    }

    #[test]
    fn area_covers_strokes_outside_viewport() {
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let image = render(&[stroke(&[(-20.0, 5.0), (5.0, 5.0)])], &[viewport.into()])
            .unwrap()
            .unwrap();
        assert_eq!(image.area, Rect::from_edges(-23.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn area_starts_at_visible_range() {
        // Zoomed in, looking at the bottom right
        let viewport = Viewport {
            x: 100.0,
            y: 50.0,
            width: 40.0,
            height: 20.0,
        };
        let image = render(
            &[stroke(&[(110.0, 60.0), (130.0, 60.0)])],
            &[viewport.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(image.area, Rect::from_edges(100.0, 50.0, 140.0, 70.0));
    }

    #[test]
    fn area_covers_whole_page_while_zoomed_in() {
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        // Zoomed in, looking only at the bottom right of the paper
        let viewport = Viewport {
            x: 50.0,
            y: 25.0,
            width: 25.0,
            height: 12.5,
        };
        let image = render(&[], &[page.into(), viewport.into()])
            .unwrap()
            .unwrap();
        assert_eq!(image.area, Rect::from_edges(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn large_area_is_scaled_down() {
        let image = render(&[stroke(&[(0.0, 0.0), (4000.0, 1000.0)])], &[])
            .unwrap()
            .unwrap();
        let (width, height, _) = decode(&image.png);
        assert_eq!(width, MAX_IMAGE_SIDE as u32);
        assert!(height < width);
        assert!(image.scale < 1.0);
    }
}
