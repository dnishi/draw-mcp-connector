//! Draws the canvas as PNG, so AI agents see strokes as an image instead of lists of coordinates.
//!
//! Line width and corner rounding match the canvas page's drawing (`radiusAt` and `smoothPoints` in
//! `src/ui/src/features/canvas/stroke.ts`). Like the page, it draws on white in each stroke's color.

use serde::Serialize;

use std::ops::Range;
use std::sync::OnceLock;

use crate::color;
use crate::geometry::{Point, closest_on_segment};
use crate::protocol::{ImagePlacement, PageSize, Stroke, StrokePoint, Viewport};

/// Line radius at pressure 1 (CSS pixels). PEN_MAX_RADIUS in stroke.ts
const PEN_MAX_RADIUS: f64 = 3.0;
/// Lower bound so lines do not vanish when pressure is near 0. MIN_PRESSURE in stroke.ts
const MIN_PRESSURE: f64 = 0.25;
/// Upper bound of the image's long side (pixels). Larger areas are drawn scaled down
const MAX_IMAGE_SIDE: f64 = 2000.0;
/// Upper limit of the longer side of an imported image (PNG) in pixels. Larger ones are not decoded.
/// The page shrinks the longer side to 2000px before sending (MAX_IMAGE_SIDE in `src/ui/src/features/canvas/imageImport.ts`).
/// Prevents a small file with large dimensions from taking gigabytes when decoded
const MAX_SOURCE_SIDE: u32 = 4096;
/// Lower limit of the radius on the image (pixels), so that lines do not vanish when drawn shrunk
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

    /// The area both overlap. None if they are apart or only touch at an edge
    pub fn intersect(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        // No overlap if the width or height is 0 (an image exactly touching the paper edge is outside the paper)
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self::from_edges(left, top, right, bottom))
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

/// A decoded PNG (width, height, RGBA8 pixels). Colors are premultiplied by alpha (`decode_png`)
type DecodedImage = (u32, u32, Vec<u8>);

/// The bytes (PNG) of an imported image and their decoded result.
/// Decoded only once, the first time it is drawn (so the AI's reads do not decode a large PNG every time)
pub struct StoredImage {
    bytes: Vec<u8>,
    /// None if it cannot be decoded or the format is not supported
    decoded: OnceLock<Option<DecodedImage>>,
}

impl StoredImage {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            decoded: OnceLock::new(),
        }
    }

    /// The PNG bytes
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The decoded result. Decoded on the first call and returned from then on
    fn decoded(&self) -> Option<&DecodedImage> {
        self.decoded
            .get_or_init(|| decode_png(&self.bytes))
            .as_ref()
    }
}

/// Decodes PNG bytes into RGBA8 (colors premultiplied by alpha). None if it cannot be decoded, the format is not supported,
/// or the longer side exceeds MAX_SOURCE_SIDE
fn decode_png(bytes: &[u8]) -> Option<DecodedImage> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .ok()?;
    // Before allocating the area to decode into, check from the header that it is not too large
    let header = reader.info();
    if header.width.max(header.height) > MAX_SOURCE_SIDE {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let mut pixels = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        png::ColorType::Grayscale => buffer
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
            .collect(),
        _ => return None,
    };
    // Premultiply colors by alpha. The RGB of a transparent pixel is an invisible color
    // with undefined contents. Interpolating without premultiplying bleeds that color into the edges
    for pixel in pixels.chunks_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in pixel[..3].iter_mut() {
            // Same value as rounding color * alpha / 255
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Some((info.width, info.height, pixels))
}

/// RGBA of the source image pixel at (x, y) (premultiplied by alpha, 0 to 1)
fn texel((width, _, rgba): &DecodedImage, x: usize, y: usize) -> [f32; 4] {
    let at = (y * *width as usize + x) * 4;
    std::array::from_fn(|c| f32::from(rgba[at + c]) / 255.0)
}

/// Averages the pixels overlapping the rectangle xs × ys (in source image pixels), weighted by the overlapping area
/// (when shrinking; no pixels in between are skipped). Parts outside the image are excluded
fn area_average(image: &DecodedImage, xs: Range<f64>, ys: Range<f64>) -> [f32; 4] {
    let (width, height, _) = image;
    let (x0, x1) = (
        xs.start.clamp(0.0, f64::from(*width)),
        xs.end.clamp(0.0, f64::from(*width)),
    );
    let (y0, y1) = (
        ys.start.clamp(0.0, f64::from(*height)),
        ys.end.clamp(0.0, f64::from(*height)),
    );
    // Length of the overlap between pixel index and [from, to)
    let overlap = |index: usize, from: f64, to: f64| {
        (to.min(index as f64 + 1.0) - from.max(index as f64)).max(0.0) as f32
    };
    let mut sum = [0.0f32; 4];
    let mut covered = 0.0f32;
    for sy in y0.floor() as usize..y1.ceil() as usize {
        let weight_y = overlap(sy, y0, y1);
        for sx in x0.floor() as usize..x1.ceil() as usize {
            let weight = weight_y * overlap(sx, x0, x1);
            if weight <= 0.0 {
                continue;
            }
            for (total, value) in sum.iter_mut().zip(texel(image, sx, sy)) {
                *total += value * weight;
            }
            covered += weight;
        }
    }
    if covered <= 0.0 {
        return [0.0; 4];
    }
    sum.map(|total| total / covered)
}

/// Bilinearly interpolates position (x, y) (in source image pixels) from the four surrounding pixels (when enlarging)
fn bilinear(image: &DecodedImage, x: f64, y: f64) -> [f32; 4] {
    let (width, height, _) = image;
    // Subtracting 0.5 makes it relative to the pixel centers
    let u = x.clamp(0.0, f64::from(*width) - 0.001) - 0.5;
    let v = y.clamp(0.0, f64::from(*height) - 0.001) - 0.5;
    let (x0, y0) = (u.max(0.0).floor() as usize, v.max(0.0).floor() as usize);
    let (x1, y1) = (
        (x0 + 1).min(*width as usize - 1),
        (y0 + 1).min(*height as usize - 1),
    );
    // At the left and top edges u / v start at -0.5. x0 / y0 are rounded to 0, so
    // unless the fractions are kept within 0 to 1 they pull in colors outside the range (from the other side) and change the color
    let (fx, fy) = (
        (u - x0 as f64).clamp(0.0, 1.0) as f32,
        (v - y0 as f64).clamp(0.0, 1.0) as f32,
    );
    let lerp = |a: [f32; 4], b: [f32; 4], t: f32| std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t);
    let upper = lerp(texel(image, x0, y0), texel(image, x1, y0), fx);
    let lower = lerp(texel(image, x0, y1), texel(image, x1, y1), fx);
    lerp(upper, lower, fy)
}

/// Draws the regions (areas that are always captured, such as the paper and the visible area) and the area containing all strokes.
/// The image (laid under the strokes) adds only its overlap with clip to the area. clip is the area visible to the user
/// (the paper of connected pages); if it is None (paper unknown), the whole image area is added.
/// The image is added to the area even if its bytes cannot be decoded. None if there is nothing
///
/// clip is used only to decide the area. Drawing already draws only the pixels within the output area
/// (`pixel_range`), so there is no need to cut it out with clip
pub fn render(
    strokes: &[Stroke],
    regions: &[Rect],
    image: Option<(&ImagePlacement, &StoredImage)>,
    clip: Option<Rect>,
) -> Result<Option<CanvasImage>, png::EncodingError> {
    // The user cannot see an image outside the paper either, so it does not widen the captured area
    let image_bounds = image
        .map(|(placement, _)| Rect {
            x: placement.x,
            y: placement.y,
            width: placement.width,
            height: placement.height,
        })
        .and_then(|bounds| match clip {
            Some(clip) => bounds.intersect(clip),
            None => Some(bounds),
        });
    let Some(area) = regions
        .iter()
        .copied()
        .chain(strokes.iter().filter_map(stroke_bounds))
        .chain(image_bounds)
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
    if let Some((placement, stored)) = image
        && let Some(decoded) = stored.decoded()
    {
        raster.draw_image(placement, decoded, area, scale);
    }
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

    /// Stretches an image (RGBA8, premultiplied by alpha) over the placement rectangle and paints it with its alpha.
    /// Drawn before the strokes so it lies underneath. When shrinking (source pixels finer than output pixels),
    /// averages the source pixels covered by each output pixel by overlapping area; when enlarging, samples bilinearly
    /// on the premultiplied colors. The pixels at the edge are faded by their coverage
    fn draw_image(
        &mut self,
        placement: &ImagePlacement,
        decoded: &DecodedImage,
        area: Rect,
        scale: f64,
    ) {
        let (width, height, _) = decoded;
        if *width == 0
            || *height == 0
            || !(placement.width.is_finite() && placement.height.is_finite())
            || placement.width <= 0.0
            || placement.height <= 0.0
        {
            return;
        }
        let rect = Rect {
            x: placement.x,
            y: placement.y,
            width: placement.width,
            height: placement.height,
        };
        let (left, top) = ((rect.x - area.x) * scale, (rect.y - area.y) * scale);
        let (right, bottom) = (
            (rect.right() - area.x) * scale,
            (rect.bottom() - area.y) * scale,
        );
        // Number of source pixels within one output pixel. Shrinks if either exceeds 1
        let downscale = f64::from(*width) / (rect.width * scale) > 1.0
            || f64::from(*height) / (rect.height * scale) > 1.0;
        // Where the coordinate of an output pixel (its top-left corner) falls in the source image (position in source pixels)
        let source_x = |px: f64| ((px / scale + area.x) - rect.x) / rect.width * f64::from(*width);
        let source_y =
            |py: f64| ((py / scale + area.y) - rect.y) / rect.height * f64::from(*height);
        // The four interpolated RGBA values at an output pixel (premultiplied by alpha, 0 to 1)
        let sample_pixel = |px: u32, py: u32| -> [f32; 4] {
            let (px, py) = (f64::from(px), f64::from(py));
            if downscale {
                // The rectangle of the source image this output pixel covers
                area_average(
                    decoded,
                    source_x(px)..source_x(px + 1.0),
                    source_y(py)..source_y(py + 1.0),
                )
            } else {
                // Center of the output pixel
                bilinear(decoded, source_x(px + 0.5), source_y(py + 0.5))
            }
        };
        for py in pixel_range(top as f32, bottom as f32, self.height) {
            // Fraction of this pixel covered by the image rectangle (the edges are faded to reduce jaggies)
            let coverage_y =
                (bottom.min(py as f64 + 1.0) - top.max(py as f64)).clamp(0.0, 1.0) as f32;
            for px in pixel_range(left as f32, right as f32, self.width) {
                let coverage = coverage_y
                    * (right.min(px as f64 + 1.0) - left.max(px as f64)).clamp(0.0, 1.0) as f32;
                let source = sample_pixel(px, py);
                let alpha = source[3] * coverage;
                if alpha <= 0.0 {
                    continue;
                }
                let cell = &mut self.rgb[py as usize * self.width as usize + px as usize];
                // The interpolated color is not divided by alpha; only the coverage is applied when blending
                // (dst = dst * (1 - α * coverage) + color * coverage)
                let fade = 1.0 - alpha;
                for (c, dst) in cell.iter_mut().enumerate() {
                    *dst = *dst * fade + source[c] * coverage;
                }
            }
        }
    }

    /// Writes into coverage the darkness of a segment with round ends whose width goes from radius a to radius b
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

    /// Rendering without an image (for the existing tests)
    fn render_no_image(
        strokes: &[Stroke],
        regions: &[Rect],
    ) -> Result<Option<CanvasImage>, png::EncodingError> {
        render(strokes, regions, None, None)
    }

    /// Encodes RGBA pixels into a PNG
    fn png_image(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        png
    }

    /// A PNG filled with one color (a is 0 to 255)
    fn solid_png(width: u32, height: u32, rgb: [u8; 3], a: u8) -> Vec<u8> {
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], a])
            .collect();
        png_image(width, height, &rgba)
    }

    fn placement(x: f64, y: f64, width: f64, height: f64) -> ImagePlacement {
        ImagePlacement {
            id: "img".into(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn image_is_drawn_below_strokes() {
        let png = StoredImage::new(solid_png(4, 2, [255, 0, 0], 255));
        let image = placement(0.0, 0.0, 40.0, 20.0);
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0,
        };
        let drawn = render(
            &[stroke(&[(20.0, 0.0), (20.0, 20.0)])],
            &[viewport.into()],
            Some((&image, &png)),
            None,
        )
        .unwrap()
        .unwrap();

        let (width, _, pixels) = decode(&drawn.png);
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(10, 10), [255, 0, 0], "画像の上は赤");
        assert_eq!(at(20, 10), [0, 0, 0], "画像より筆跡が上");
        assert_eq!(at(35, 15), [255, 0, 0], "筆跡の無い所も赤");
    }

    #[test]
    fn image_area_is_included_even_without_strokes() {
        let png = StoredImage::new(solid_png(2, 2, [0, 0, 255], 255));
        let image = placement(10.0, 10.0, 8.0, 4.0);
        let drawn = render(&[], &[], Some((&image, &png)), None)
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, Rect::from_edges(10.0, 10.0, 18.0, 14.0));
        let (width, height, pixels) = decode(&drawn.png);
        assert_eq!((width, height), (8, 4));
        assert_eq!(&pixels[..3], [0, 0, 255], "引き伸ばした青");
        assert_eq!(&pixels[((3 * width as usize + 7) * 3)..][..3], [0, 0, 255]);
    }

    #[test]
    fn intersect_is_the_overlap_or_none() {
        let page = Rect::from_edges(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            Rect::from_edges(5.0, 5.0, 20.0, 20.0).intersect(page),
            Some(Rect::from_edges(5.0, 5.0, 10.0, 10.0))
        );
        // Touching only at an edge or being apart is not an overlap
        assert_eq!(
            Rect::from_edges(10.0, 0.0, 20.0, 10.0).intersect(page),
            None
        );
        assert_eq!(
            Rect::from_edges(30.0, 30.0, 40.0, 40.0).intersect(page),
            None
        );
    }

    /// Even an image larger than the paper keeps the captured area to the paper (the user cannot see outside the paper either)
    #[test]
    fn clip_keeps_the_area_to_the_page() {
        let png = StoredImage::new(solid_png(2, 2, [0, 0, 255], 255));
        // An image larger than the paper (0..40, 0..20), extending far beyond the right and bottom
        let image = placement(20.0, 10.0, 4000.0, 3000.0);
        let page = Rect::from_edges(0.0, 0.0, 40.0, 20.0);
        let drawn = render(&[], &[page], Some((&image, &png)), Some(page))
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, page, "写す範囲は紙のまま");
        assert_eq!(drawn.scale, 1.0, "紙の外の画像で全体が縮まない");

        let (width, _, pixels) = decode(&drawn.png);
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(30, 15), [0, 0, 255], "紙と重なる部分は青");
        assert_eq!(at(5, 5), [255, 255, 255], "画像より外は白");
    }

    #[test]
    fn image_outside_the_clip_leaves_the_area_and_the_pixels() {
        let png = StoredImage::new(solid_png(2, 2, [0, 0, 255], 255));
        // An image placed apart to the right of the paper (not even touching the edge)
        let image = placement(60.0, 0.0, 20.0, 20.0);
        let page = Rect::from_edges(0.0, 0.0, 40.0, 20.0);
        let drawn = render(&[], &[page], Some((&image, &png)), Some(page))
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, page, "紙の外の画像は範囲を広げない");
        let (_, _, pixels) = decode(&drawn.png);
        assert!(
            pixels.chunks(3).all(|p| p == [255, 255, 255]),
            "紙の外の画像は描かない"
        );
    }

    #[test]
    fn without_clip_the_whole_image_is_in_the_area() {
        // When no page is connected and the paper is unknown, the whole image area is captured
        let png = StoredImage::new(solid_png(2, 2, [0, 0, 255], 255));
        let image = placement(20.0, 0.0, 200.0, 20.0);
        let page = Rect::from_edges(0.0, 0.0, 40.0, 20.0);
        let drawn = render(&[], &[page], Some((&image, &png)), None)
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, Rect::from_edges(0.0, 0.0, 220.0, 20.0));
    }

    #[test]
    fn stretched_image_keeps_the_edge_colors() {
        // Stretches a 2×1 image with different left and right colors 4 times. The edge pixels must not face outside the interpolation
        let png = StoredImage::new(png_image(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]));
        let image = placement(0.0, 0.0, 8.0, 4.0);
        let drawn = render(&[], &[], Some((&image, &png)), None)
            .unwrap()
            .unwrap();
        let (width, _, pixels) = decode(&drawn.png);
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(0, 0), [255, 0, 0], "左端・上端は左の色");
        assert_eq!(at(0, 2), [255, 0, 0], "左端は左の色のまま");
        assert_eq!(at(7, 2), [0, 0, 255], "右端は右の色");
    }

    #[test]
    fn transparent_edge_mixes_toward_the_background() {
        // Stretches a 2×1 image (opaque red on the left, fully transparent black on the right) 4 times over a white background.
        // Interpolating on premultiplied colors keeps the transparent black from bleeding into the edge
        let png = StoredImage::new(png_image(2, 1, &[255, 0, 0, 255, 0, 0, 0, 0]));
        let image = placement(0.0, 0.0, 8.0, 4.0);
        let drawn = render(&[], &[], Some((&image, &png)), None)
            .unwrap()
            .unwrap();
        let (width, _, pixels) = decode(&drawn.png);
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(0, 0), [255, 0, 0], "左端は赤のまま");
        // The boundary is red laid over white at the alpha ratio (without premultiplying, the transparent black mixes in and it darkens like [195, 96, 96])
        assert_eq!(at(3, 0), [255, 96, 96], "境目は赤と白地の間の色");
        assert_eq!(at(5, 0), [255, 223, 223], "透明に近いほど白地へ近づく");
        assert_eq!(at(7, 0), [255, 255, 255], "α が 0 の画素は白地");
    }

    #[test]
    fn oversized_png_is_not_decoded() {
        // A PNG whose longer side exceeds the limit is not decoded (it is still included in the area)
        let wide = StoredImage::new(solid_png(MAX_SOURCE_SIDE + 1, 1, [255, 0, 0], 255));
        assert_eq!(wide.decoded(), None);
        let image = placement(0.0, 0.0, 8.0, 4.0);
        let drawn = render(&[], &[], Some((&image, &wide)), None)
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, Rect::from_edges(0.0, 0.0, 8.0, 4.0));
        assert_eq!(
            &decode(&drawn.png).2[..3],
            [255, 255, 255],
            "描かれず白地のまま"
        );

        // Exactly at the limit is decoded
        let edge = StoredImage::new(solid_png(MAX_SOURCE_SIDE, 1, [255, 0, 0], 255));
        assert!(edge.decoded().is_some());
    }

    /// When shrinking, the source pixels covered by each output pixel are averaged by overlapping area
    #[test]
    fn downscaled_image_averages_the_covered_pixels() {
        // Shrinks a white 8×8 with a 1px-wide black vertical line every 4px to 1/4.
        // Every output pixel covers 4 source columns (1 black : 3 white), so all columns become the same gray.
        // Bilinear sampling skips the lines, so some columns come out pure white
        let rgba: Vec<u8> = (0..8 * 8)
            .flat_map(|i| {
                let line = i % 8 % 4 == 0;
                if line {
                    [0, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            })
            .collect();
        let png = StoredImage::new(png_image(8, 8, &rgba));
        let image = placement(0.0, 0.0, 2.0, 2.0);
        let drawn = render(&[], &[], Some((&image, &png)), None)
            .unwrap()
            .unwrap();

        let (width, height, pixels) = decode(&drawn.png);
        assert_eq!((width, height), (2, 2), "1/4 に縮む");
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        // Average of 3 white : 1 black, (3 * 255) / 4 = 191.25
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(at(x, y), [191, 191, 191], "どの列も同じ灰色");
            }
        }
    }

    #[test]
    fn transparent_and_undecodable_images_leave_the_background() {
        let transparent = StoredImage::new(solid_png(2, 2, [0, 0, 0], 0));
        let image = placement(0.0, 0.0, 8.0, 4.0);
        let drawn = render(&[], &[], Some((&image, &transparent)), None)
            .unwrap()
            .unwrap();
        let (_, _, pixels) = decode(&drawn.png);
        assert_eq!(&pixels[..3], [255, 255, 255], "α0 は白のまま");

        // Even bytes that cannot be decoded have the image area captured
        let broken = StoredImage::new(b"not a png".to_vec());
        let drawn = render(&[], &[], Some((&image, &broken)), None)
            .unwrap()
            .unwrap();
        assert_eq!(drawn.area, Rect::from_edges(0.0, 0.0, 8.0, 4.0));
        assert_eq!(
            decode(&drawn.png).1,
            4,
            "描画はされなくても範囲は画像を覆う"
        );
    }

    #[test]
    fn stored_image_is_decoded_once_and_drawn_the_same() {
        let stored = StoredImage::new(solid_png(2, 2, [0, 128, 0], 255));
        let image = placement(0.0, 0.0, 8.0, 4.0);
        assert!(stored.decoded.get().is_none(), "描くまでは読まない");

        let first = render(&[], &[], Some((&image, &stored)), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.decoded.get(),
            Some(&Some((2, 2, [0, 128, 0, 255].repeat(4)))),
            "読んだ結果は要素の中に入る"
        );
        let second = render(&[], &[], Some((&image, &stored)), None)
            .unwrap()
            .unwrap();
        assert_eq!(first.png, second.png, "2 回目も同じ画像になる");
        assert_eq!(first.area, second.area);
    }

    /// Decodes a PNG back into (width, height, RGB pixels)
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
        assert!(render_no_image(&[], &[]).unwrap().is_none());
    }

    #[test]
    fn draws_black_line_on_white_viewport() {
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0,
        };
        let image = render_no_image(&[stroke(&[(5.0, 10.0), (35.0, 10.0)])], &[viewport.into()])
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
        let image = render_no_image(&[black, red], &[Rect::from_edges(0.0, 0.0, 40.0, 20.0)])
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
        let image = render_no_image(&[stroke(&[(-20.0, 5.0), (5.0, 5.0)])], &[viewport.into()])
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
        let image = render_no_image(
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
        let image = render_no_image(&[], &[page.into(), viewport.into()])
            .unwrap()
            .unwrap();
        assert_eq!(image.area, Rect::from_edges(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn large_area_is_scaled_down() {
        let image = render_no_image(&[stroke(&[(0.0, 0.0), (4000.0, 1000.0)])], &[])
            .unwrap()
            .unwrap();
        let (width, height, _) = decode(&image.png);
        assert_eq!(width, MAX_IMAGE_SIDE as u32);
        assert!(height < width);
        assert!(image.scale < 1.0);
    }
}
