//! How the canvas is presented to AI agents.
//!
//! Stroke point lists are not returned (they are many and unsuited to reading what was written).
//! Instead it returns a canvas image (PNG) and each stroke's ID, author, and bounds.
//! When points are needed, read them with read_strokes by ID.
//! Bounds are integers rounded outward (the image given to the AI is never finer than 1 CSS pixel,
//! and decimal digits only spend tokens).

use rmcp::ErrorData;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;

use crate::canvas::{CanvasView, ChangesSince, Snapshot};
use crate::protocol::{Author, CanvasOp, Stroke};
use crate::render::{self, Rect};

/// Bounds in integer CSS pixels
#[derive(Debug, PartialEq, Serialize)]
struct PixelBounds {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

impl From<Rect> for PixelBounds {
    /// Rounds the bounds outward (so ink never sticks out)
    fn from(rect: Rect) -> Self {
        let left = rect.x.floor() as i64;
        let top = rect.y.floor() as i64;
        let right = (rect.x + rect.width).ceil() as i64;
        let bottom = (rect.y + rect.height).ceil() as i64;
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

#[derive(Serialize)]
struct StrokeSummary {
    id: String,
    author: Author,
    color: String,
    /// Area covered by ink (including line width). Absent for strokes with no points
    #[serde(skip_serializing_if = "Option::is_none")]
    bounds: Option<PixelBounds>,
}

impl From<&Stroke> for StrokeSummary {
    fn from(stroke: &Stroke) -> Self {
        Self {
            id: stroke.id.clone(),
            author: stroke.author,
            color: stroke.color.clone(),
            bounds: render::stroke_bounds(stroke).map(PixelBounds::from),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum OpSummary {
    Insert { stroke: StrokeSummary, index: usize },
    Remove { ids: Vec<String> },
    Replace { stroke: StrokeSummary },
}

impl From<&CanvasOp> for OpSummary {
    fn from(op: &CanvasOp) -> Self {
        match op {
            CanvasOp::Insert { stroke, index } => Self::Insert {
                stroke: stroke.into(),
                index: *index,
            },
            CanvasOp::Remove { ids } => Self::Remove { ids: ids.clone() },
            CanvasOp::Replace { stroke } => Self::Replace {
                stroke: stroke.into(),
            },
        }
    }
}

/// Which part of which canvas revision the accompanying image shows
#[derive(Serialize)]
struct ImageInfo {
    revision: u64,
    /// The image's top left is canvas (x, y). width and height are canvas CSS pixels
    #[serde(flatten)]
    area: Rect,
    /// Image pixels per canvas CSS pixel
    scale: f64,
}

#[derive(Serialize)]
struct CanvasSummary {
    revision: u64,
    /// Per connected canvas page: the paper size (the paper starts at (0, 0)) and the visible area
    canvases: Vec<CanvasView>,
    /// No image is returned when there is nothing to draw
    image: Option<ImageInfo>,
    /// Stacking order (later is on top). Absent when asked to omit it
    #[serde(skip_serializing_if = "Option::is_none")]
    strokes: Option<Vec<StrokeSummary>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ChangesSummary {
    Ops {
        revision: u64,
        /// Pages connected at the time of the call and their visible areas. Visible-area changes do not bump
        /// the revision, so they are always returned separately from the diff
        canvases: Vec<CanvasView>,
        ops: Vec<OpSummary>,
        /// No image is returned when nothing changed (the same image as last time is not sent)
        #[serde(skip_serializing_if = "Option::is_none")]
        image: Option<ImageInfo>,
    },
    Snapshot(CanvasSummary),
}

/// Canvas image. Returns (image description, PNG)
fn render_image(snapshot: &Snapshot) -> Result<Option<(ImageInfo, Vec<u8>)>, ErrorData> {
    // The whole paper and visible area of each connected page are included even with no strokes
    let regions: Vec<Rect> = snapshot
        .views
        .iter()
        .flat_map(|view| {
            view.page
                .map(Rect::from)
                .into_iter()
                .chain([Rect::from(view.viewport)])
        })
        .collect();
    let image = render::render(&snapshot.strokes, &regions)
        .map_err(|error| ErrorData::internal_error(format!("画像を作れない: {error}"), None))?;
    Ok(image.map(|image| {
        let info = ImageInfo {
            revision: snapshot.revision,
            area: image.area,
            scale: image.scale,
        };
        (info, image.png)
    }))
}

pub fn json_block(value: &impl Serialize) -> Result<ContentBlock, ErrorData> {
    let text = serde_json::to_string(value)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(ContentBlock::text(text))
}

/// Returns the description (JSON) and, if any, the image
fn result_with_image(
    summary: &impl Serialize,
    png: Option<Vec<u8>>,
) -> Result<CallToolResult, ErrorData> {
    let mut content = vec![json_block(summary)?];
    if let Some(png) = png {
        content.push(ContentBlock::image(base64(&png), "image/png"));
    }
    Ok(CallToolResult::success(content))
}

fn canvas_summary(
    snapshot: &Snapshot,
    include_strokes: bool,
) -> Result<(CanvasSummary, Option<Vec<u8>>), ErrorData> {
    let (image, png) = render_image(snapshot)?.unzip();
    let summary = CanvasSummary {
        revision: snapshot.revision,
        canvases: snapshot.views.clone(),
        image,
        strokes: include_strokes
            .then(|| snapshot.strokes.iter().map(StrokeSummary::from).collect()),
    };
    Ok((summary, png))
}

/// Returns the description and image of the whole canvas. If include_strokes is false, omits the per-stroke list
pub fn canvas_result(
    snapshot: &Snapshot,
    include_strokes: bool,
) -> Result<CallToolResult, ErrorData> {
    let (summary, png) = canvas_summary(snapshot, include_strokes)?;
    result_with_image(&summary, png)
}

/// Returns the description of the diff and the current canvas image. No image is attached when nothing changed
pub fn changes_result(
    changes: &ChangesSince,
    current: &Snapshot,
) -> Result<CallToolResult, ErrorData> {
    let (summary, png) = match changes {
        ChangesSince::Ops { revision, ops } => {
            let (image, png) = if ops.is_empty() {
                (None, None)
            } else {
                render_image(current)?.unzip()
            };
            let summary = ChangesSummary::Ops {
                revision: *revision,
                canvases: current.views.clone(),
                ops: ops.iter().map(OpSummary::from).collect(),
                image,
            };
            (summary, png)
        }
        ChangesSince::Snapshot(snapshot) => {
            // When a diff cannot be returned, return the whole canvas. It replaces the diff, so the stroke list is not omitted
            let (summary, png) = canvas_summary(snapshot, true)?;
            (ChangesSummary::Snapshot(summary), png)
        }
    };
    result_with_image(&summary, png)
}

/// Base64 (RFC 4648 section 4, padded with =). MCP images are sent as Base64 text
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let group = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        // Turn 3 bytes into 4 characters of 6 bits each. Missing bytes become =
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(group >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PageSize, StrokePoint, Viewport};

    fn stroke(id: &str) -> Stroke {
        Stroke {
            id: id.into(),
            author: Author::Ai,
            color: crate::color::BLACK.into(),
            points: vec![
                StrokePoint {
                    x: 10.0,
                    y: 10.0,
                    pressure: 1.0,
                },
                StrokePoint {
                    x: 20.0,
                    y: 10.0,
                    pressure: 1.0,
                },
            ],
        }
    }

    /// Page of connection id with a width × height screen (whole page visible at 100%)
    fn view(id: u64, width: f64, height: f64) -> CanvasView {
        CanvasView {
            id,
            ip: std::net::IpAddr::from([192, 168, 0, id as u8]),
            page: Some(PageSize { width, height }),
            viewport: Viewport {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            revision: 4,
            views: vec![view(2, 30.0, 20.0)],
            strokes: vec![stroke("a")],
        }
    }

    /// JSON of canvases in snapshot()
    fn canvases_json() -> serde_json::Value {
        serde_json::json!([{
            "id": 2,
            "ip": "192.168.0.2",
            "page": { "width": 30.0, "height": 20.0 },
            "viewport": { "x": 0.0, "y": 0.0, "width": 30.0, "height": 20.0 }
        }])
    }

    fn summary_json(result: &CallToolResult) -> serde_json::Value {
        let text = result.content[0].as_text().unwrap();
        serde_json::from_str(&text.text).unwrap()
    }

    #[test]
    fn base64_matches_rfc4648_test_vectors() {
        // RFC 4648 section 10
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn canvas_result_has_summary_and_png_without_points() {
        let result = canvas_result(&snapshot(), true).unwrap();
        assert_eq!(result.content.len(), 2);
        let image = result.content[1].as_image().unwrap();
        assert_eq!(image.mime_type, "image/png");

        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "revision": 4,
                "canvases": canvases_json(),
                "image": { "revision": 4, "x": 0.0, "y": 0.0, "width": 30.0, "height": 20.0, "scale": 1.0 },
                "strokes": [{
                    "id": "a",
                    "author": "ai",
                    "color": "#000000",
                    "bounds": { "x": 7, "y": 7, "width": 16, "height": 6 }
                }]
            })
        );
    }

    #[test]
    fn bounds_are_rounded_outward() {
        let rect = Rect {
            x: 1010.1376748177948,
            y: -0.5,
            width: 2.201356193530728,
            height: 9.0,
        };
        // Right edge 1012.339… rounds up to 1013, bottom edge 8.5 to 9
        assert_eq!(
            PixelBounds::from(rect),
            PixelBounds {
                x: 1010,
                y: -1,
                width: 3,
                height: 10,
            }
        );
    }

    /// With two devices of different sizes connected, shows the area containing both papers and returns both views
    #[test]
    fn every_connected_canvas_is_described_and_imaged() {
        let both = Snapshot {
            revision: 4,
            views: vec![view(1, 30.0, 20.0), view(2, 50.0, 10.0)],
            strokes: Vec::new(),
        };
        let json = summary_json(&canvas_result(&both, true).unwrap());
        assert_eq!(json["canvases"].as_array().unwrap().len(), 2);
        assert_eq!(json["canvases"][1]["ip"], "192.168.0.2");
        assert_eq!(
            json["image"],
            serde_json::json!({ "revision": 4, "x": 0.0, "y": 0.0, "width": 50.0, "height": 20.0, "scale": 1.0 })
        );
    }

    #[test]
    fn strokes_can_be_omitted() {
        let result = canvas_result(&snapshot(), false).unwrap();
        assert_eq!(result.content.len(), 2, "画像は返す");
        let json = summary_json(&result);
        assert!(json.get("strokes").is_none());
        assert_eq!(json["revision"], 4);
    }

    #[test]
    fn empty_canvas_has_no_image() {
        let empty = Snapshot {
            revision: 0,
            views: Vec::new(),
            strokes: Vec::new(),
        };
        let result = canvas_result(&empty, true).unwrap();
        assert_eq!(result.content.len(), 1);
        assert_eq!(summary_json(&result)["image"], serde_json::Value::Null);
    }

    #[test]
    fn changes_are_summarized() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: vec![
                CanvasOp::Insert {
                    stroke: stroke("a"),
                    index: 0,
                },
                CanvasOp::Remove {
                    ids: vec!["b".into()],
                },
            ],
        };
        let result = changes_result(&changes, &snapshot()).unwrap();
        let json = summary_json(&result);
        assert_eq!(json["kind"], "ops");
        assert_eq!(json["ops"][0]["type"], "insert");
        assert_eq!(json["ops"][0]["stroke"]["id"], "a");
        assert!(json["ops"][0]["stroke"].get("points").is_none());
        assert_eq!(
            json["ops"][1],
            serde_json::json!({ "type": "remove", "ids": ["b"] })
        );
        assert_eq!(json["image"]["revision"], 4);
        assert_eq!(result.content.len(), 2);
    }

    #[test]
    fn no_changes_return_revision_and_viewport() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: Vec::new(),
        };
        let result = changes_result(&changes, &snapshot()).unwrap();
        assert_eq!(result.content.len(), 1, "画像は付けない");
        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "kind": "ops",
                "revision": 4,
                "canvases": canvases_json(),
                "ops": []
            })
        );
    }
}
