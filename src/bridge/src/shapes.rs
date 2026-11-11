//! Turns shapes the AI writes with draw_shapes (polylines, rectangles, ellipses, text) into stroke point lists.
//!
//! This spares the AI from sending point lists (they are long and use many tokens each time).
//! The canvas page rounds corners with Chaikin's algorithm before drawing strokes (`smoothPoints` in `stroke.ts`).
//! To keep shapes and characters from losing their form, interior points are inserted twice (a segment between identical points does not move when rounded).

use std::f64::consts::TAU;

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use crate::glyphs::{self, EM};
use crate::protocol::StrokePoint;

/// Upper limit of the text size (size) (CSS pixels)
const MAX_TEXT_SIZE: f64 = 1000.0;
/// Upper limit of characters written at once
const MAX_TEXT_CHARS: usize = 2000;
/// Line height (ratio to the text size)
const LINE_HEIGHT: f64 = 1.2;
/// Spacing between alphanumerics (characters packed to their width) (glyph frame units)
const LATIN_GAP: f64 = 14.0;
/// Width of a half-width space (glyph frame units)
const SPACE_ADVANCE: f64 = 35.0;
/// Approximate segment length when turning an ellipse into a polyline (CSS pixels)
const ELLIPSE_STEP: f64 = 4.0;
/// Default pressure of shapes
const DEFAULT_SHAPE_PRESSURE: f64 = 0.5;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ShapeKind {
    /// Polyline (2 points or more). Making the last point equal to the first closes it into a polygon
    Line { points: Vec<Position> },
    /// Rectangle. (x, y) is the top left
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    /// Ellipse. (cx, cy) is the center; rx and ry are the horizontal and vertical radii
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    /// Text. (x, y) is the top left of the first line. size is the glyph frame size (width and height of a full-width character);
    /// line spacing is 1.2 times size. \n breaks lines. Alphanumerics are packed to their width
    Text {
        text: String,
        x: f64,
        y: f64,
        size: f64,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Shape {
    #[serde(flatten)]
    pub kind: ShapeKind,
    /// Line color ("#rrggbb"). Black if omitted
    pub color: Option<String>,
    /// Pressure (0 to 1), which sets the line width. If omitted, 0.5 for shapes and a width matching the size for text
    pub pressure: Option<f64>,
}

/// A shape turned into point lists
#[derive(Debug, Default)]
pub struct Outline {
    /// Point list of each stroke
    pub strokes: Vec<Vec<StrokePoint>>,
    /// Characters that could not be written for lack of a glyph (their space is left blank). Without duplicates
    pub missing: Vec<char>,
}

type Polyline = Vec<(f64, f64)>;

fn check_finite(values: &[f64]) -> Result<(), String> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err("有限でない数がある".into())
    }
}

fn check_not_negative(name: &str, value: f64) -> Result<(), String> {
    if value < 0.0 {
        return Err(format!("{name} は 0 以上にする: {value}"));
    }
    Ok(())
}

/// Pressure matching the text size. 0.4 at size 30px (line width 2.4px)
fn text_pressure(size: f64) -> f64 {
    (size / 75.0).clamp(0.0, 1.0)
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Polyline {
    let (right, bottom) = (x + width, y + height);
    vec![(x, y), (right, y), (right, bottom), (x, bottom), (x, y)]
}

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Polyline {
    // Approximate perimeter (circumference of a circle with the root-mean-square radius)
    let perimeter = TAU * ((rx * rx + ry * ry) / 2.0).sqrt();
    let segments = ((perimeter / ELLIPSE_STEP).ceil() as usize).clamp(16, 360);
    (0..=segments)
        .map(|i| {
            let angle = TAU * i as f64 / segments as f64;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}

/// Turns text into polylines of strokes. Characters without a glyph go into missing and leave half a full-width space
fn text(text: &str, x: f64, y: f64, size: f64, missing: &mut Vec<char>) -> Vec<Polyline> {
    let scale = size / EM;
    let mut strokes = Vec::new();
    for (line_index, line) in text.split('\n').enumerate() {
        let top = line_index as f64 * EM * LINE_HEIGHT;
        let mut pen = 0.0; // Position from the start of the line (glyph frame units)
        for c in line.chars().filter(|&c| c != '\r') {
            if c == ' ' {
                pen += SPACE_ADVANCE;
                continue;
            }
            if c == '\u{3000}' {
                pen += EM;
                continue;
            }
            let Some(glyph) = glyphs::glyph(c) else {
                if !missing.contains(&c) {
                    missing.push(c);
                }
                pen += EM / 2.0;
                continue;
            };
            // Alphanumerics are packed to their width; other characters (kana, kanji, etc.) are placed by the frame width
            let (shift, advance) = if c.is_ascii() {
                let xs = glyph.iter().flatten().map(|&(gx, _)| gx);
                let left = xs.clone().fold(f64::INFINITY, f64::min);
                let right = xs.fold(f64::NEG_INFINITY, f64::max);
                (pen - left + LATIN_GAP / 2.0, right - left + LATIN_GAP)
            } else {
                (pen, EM)
            };
            strokes.extend(glyph.into_iter().map(|stroke| {
                stroke
                    .into_iter()
                    .map(|(gx, gy)| (x + (gx + shift) * scale, y + (gy + top) * scale))
                    .collect()
            }));
            pen += advance;
        }
    }
    strokes
}

/// Inserts interior points twice so the polyline keeps its shape when rounded with Chaikin's algorithm
fn keep_corners(polyline: Polyline, pressure: f64) -> Vec<StrokePoint> {
    let last = polyline.len().saturating_sub(1);
    polyline
        .into_iter()
        .enumerate()
        .flat_map(|(i, (x, y))| {
            let point = StrokePoint { x, y, pressure };
            let copies = if i == 0 || i == last { 1 } else { 2 };
            std::iter::repeat_n(point, copies)
        })
        .collect()
}

/// Validates a shape and turns it into stroke point lists
pub fn outline(shape: &Shape) -> Result<Outline, String> {
    let mut missing = Vec::new();
    let (polylines, default_pressure) = match &shape.kind {
        ShapeKind::Line { points } => {
            if points.len() < 2 {
                return Err("line の points は 2 点以上にする".into());
            }
            let polyline: Polyline = points.iter().map(|p| (p.x, p.y)).collect();
            check_finite(
                &polyline
                    .iter()
                    .flat_map(|&(x, y)| [x, y])
                    .collect::<Vec<_>>(),
            )?;
            (vec![polyline], DEFAULT_SHAPE_PRESSURE)
        }
        &ShapeKind::Rect {
            x,
            y,
            width,
            height,
        } => {
            check_finite(&[x, y, width, height])?;
            check_not_negative("width", width)?;
            check_not_negative("height", height)?;
            (vec![rect(x, y, width, height)], DEFAULT_SHAPE_PRESSURE)
        }
        &ShapeKind::Ellipse { cx, cy, rx, ry } => {
            check_finite(&[cx, cy, rx, ry])?;
            check_not_negative("rx", rx)?;
            check_not_negative("ry", ry)?;
            (vec![ellipse(cx, cy, rx, ry)], DEFAULT_SHAPE_PRESSURE)
        }
        ShapeKind::Text {
            text: content,
            x,
            y,
            size,
        } => {
            check_finite(&[*x, *y, *size])?;
            if !(*size > 0.0 && *size <= MAX_TEXT_SIZE) {
                return Err(format!(
                    "size は 0 より大きく {MAX_TEXT_SIZE} 以下にする: {size}"
                ));
            }
            if content.chars().count() > MAX_TEXT_CHARS {
                return Err(format!("text は {MAX_TEXT_CHARS} 文字までにする"));
            }
            (
                text(content, *x, *y, *size, &mut missing),
                text_pressure(*size),
            )
        }
    };

    let pressure = shape.pressure.unwrap_or(default_pressure);
    check_finite(&[pressure])?;
    let pressure = pressure.clamp(0.0, 1.0);
    Ok(Outline {
        strokes: polylines
            .into_iter()
            .map(|polyline| keep_corners(polyline, pressure))
            .collect(),
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: ShapeKind) -> Shape {
        Shape {
            kind,
            color: None,
            pressure: None,
        }
    }

    fn xy(points: &[StrokePoint]) -> Vec<(f64, f64)> {
        points.iter().map(|p| (p.x, p.y)).collect()
    }

    #[test]
    fn rect_doubles_corners_and_closes() {
        let outline = outline(&shape(ShapeKind::Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        }))
        .unwrap();
        assert_eq!(outline.strokes.len(), 1);
        assert_eq!(
            xy(&outline.strokes[0]),
            vec![
                (10.0, 20.0),
                (40.0, 20.0),
                (40.0, 20.0),
                (40.0, 60.0),
                (40.0, 60.0),
                (10.0, 60.0),
                (10.0, 60.0),
                (10.0, 20.0),
            ]
        );
        assert!(outline.strokes[0].iter().all(|p| p.pressure == 0.5));
    }

    #[test]
    fn ellipse_is_closed_and_on_the_curve() {
        let outline = outline(&shape(ShapeKind::Ellipse {
            cx: 50.0,
            cy: 50.0,
            rx: 20.0,
            ry: 10.0,
        }))
        .unwrap();
        let points = &outline.strokes[0];
        let (first, last) = (points.first().unwrap(), points.last().unwrap());
        assert!((first.x - last.x).abs() < 1e-9 && (first.y - last.y).abs() < 1e-9);
        for p in points {
            let on_curve = ((p.x - 50.0) / 20.0).powi(2) + ((p.y - 50.0) / 10.0).powi(2);
            assert!((on_curve - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn line_needs_two_points_and_finite_numbers() {
        let one = shape(ShapeKind::Line {
            points: vec![Position { x: 0.0, y: 0.0 }],
        });
        assert!(outline(&one).is_err());
        let infinite = shape(ShapeKind::Line {
            points: vec![
                Position { x: 0.0, y: 0.0 },
                Position {
                    x: f64::INFINITY,
                    y: 0.0,
                },
            ],
        });
        assert!(outline(&infinite).is_err());
    }

    #[test]
    fn negative_size_is_rejected() {
        let rect = shape(ShapeKind::Rect {
            x: 0.0,
            y: 0.0,
            width: -1.0,
            height: 1.0,
        });
        assert!(outline(&rect).is_err());
        let text = shape(ShapeKind::Text {
            text: "a".into(),
            x: 0.0,
            y: 0.0,
            size: 0.0,
        });
        assert!(outline(&text).is_err());
    }

    fn text_shape(content: &str, size: f64) -> Shape {
        shape(ShapeKind::Text {
            text: content.into(),
            x: 100.0,
            y: 200.0,
            size,
        })
    }

    fn horizontal_extent(strokes: &[Vec<StrokePoint>]) -> (f64, f64) {
        let xs = strokes.iter().flatten().map(|p| p.x);
        (
            xs.clone().fold(f64::INFINITY, f64::min),
            xs.fold(f64::NEG_INFINITY, f64::max),
        )
    }

    #[test]
    fn text_uses_glyph_strokes_in_the_box() {
        // 日 has 4 strokes. At size 109, glyph frame coordinates are the offsets from (x, y) as is
        let outline = outline(&text_shape("日", EM)).unwrap();
        assert_eq!(outline.strokes.len(), 4);
        assert!(outline.missing.is_empty());
        for p in outline.strokes.iter().flatten() {
            assert!((100.0..=100.0 + EM).contains(&p.x));
            assert!((200.0..=200.0 + EM).contains(&p.y));
        }
    }

    #[test]
    fn full_width_chars_advance_by_the_box_and_newline_moves_down() {
        let size = 50.0;
        let one = outline(&text_shape("日", size)).unwrap();
        let two = outline(&text_shape("日日", size)).unwrap();
        let (left, _) = horizontal_extent(&one.strokes);
        let (second_left, _) = horizontal_extent(&two.strokes[4..]);
        assert!((second_left - left - size).abs() < 1e-9);

        let lines = outline(&text_shape("日\n日", size)).unwrap();
        let dy = lines.strokes[4][0].y - lines.strokes[0][0].y;
        assert!((dy - size * LINE_HEIGHT).abs() < 1e-9);
    }

    #[test]
    fn latin_chars_are_packed_by_width() {
        // i is narrow, so the next character comes closer than a full-width frame
        let size = EM;
        let outline = outline(&text_shape("ii", size)).unwrap();
        let first = horizontal_extent(&outline.strokes[..2]);
        let second = horizontal_extent(&outline.strokes[2..]);
        let pitch = second.0 - first.0;
        assert!(pitch > 0.0 && pitch < EM / 2.0, "pitch = {pitch}");
    }

    #[test]
    fn missing_glyphs_are_reported_once_and_leave_a_gap() {
        let outline = outline(&text_shape("\u{1F600}日\u{1F600}", EM)).unwrap();
        assert_eq!(outline.missing, vec!['\u{1F600}']);
        let (left, _) = horizontal_extent(&outline.strokes);
        assert!(left >= 100.0 + EM / 2.0);
    }

    #[test]
    fn pressure_defaults_follow_the_kind() {
        let text = outline(&text_shape("a", 30.0)).unwrap();
        assert!(
            text.strokes
                .iter()
                .flatten()
                .all(|p| (p.pressure - 0.4).abs() < 1e-9)
        );

        let mut custom = text_shape("a", 30.0);
        custom.pressure = Some(2.0);
        let custom = outline(&custom).unwrap();
        assert!(custom.strokes.iter().flatten().all(|p| p.pressure == 1.0));
    }

    #[test]
    fn shape_json_has_type_and_common_fields() {
        let shape: Shape = serde_json::from_value(serde_json::json!({
            "type": "text", "text": "A", "x": 1, "y": 2, "size": 30, "color": "#ff0000"
        }))
        .unwrap();
        assert!(matches!(shape.kind, ShapeKind::Text { .. }));
        assert_eq!(shape.color.as_deref(), Some("#ff0000"));
    }
}
