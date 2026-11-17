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
use crate::protocol::{Author, CanvasOp, ImagePlacement, Sheet, Stroke};
use crate::render::{self, Rect, StoredImage};

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

/// Description of the image placed under the strokes. The original pixels are read by ID with read_image
#[derive(Serialize)]
struct PlacedImage {
    id: String,
    /// Area the image covers (the placement itself, as integers rounded outward)
    bounds: PixelBounds,
}

impl From<&ImagePlacement> for PlacedImage {
    fn from(image: &ImagePlacement) -> Self {
        Self {
            id: image.id.clone(),
            bounds: PixelBounds::from(Rect {
                x: image.x,
                y: image.y,
                width: image.width,
                height: image.height,
            }),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum OpSummary {
    Insert {
        /// Number of the changed sheet (1-based, in order). Omitted if the sheet no longer exists
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
        stroke: StrokeSummary,
        index: usize,
    },
    Remove {
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
        ids: Vec<String>,
    },
    Replace {
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
        stroke: StrokeSummary,
    },
    /// Places, moves or resizes the image (always under the strokes)
    SetImage {
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
        image: PlacedImage,
    },
    /// Removes the image
    RemoveImage {
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
    },
    /// Inserts an empty sheet
    AddSheet {
        /// Number of the inserted sheet. Omitted if it no longer exists
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
    },
    /// Removes a sheet with its contents
    RemoveSheet,
    /// Changes the current sheet
    SelectSheet {
        /// Number of the sheet switched to. Omitted if it no longer exists
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<usize>,
    },
    /// Removes all sheets, leaving one blank sheet
    ResetSheets,
}

/// Summarizes one change. Sheet numbers are looked up from the current state, so
/// changes to sheets that no longer exist have no sheet
fn summarize(op: &CanvasOp, current: &Snapshot) -> OpSummary {
    let number = |sheet: &str| current.sheet_number(sheet);
    match op {
        CanvasOp::Insert {
            sheet,
            stroke,
            index,
        } => OpSummary::Insert {
            sheet: number(sheet),
            stroke: stroke.into(),
            index: *index,
        },
        CanvasOp::Remove { sheet, ids } => OpSummary::Remove {
            sheet: number(sheet),
            ids: ids.clone(),
        },
        CanvasOp::Replace { sheet, stroke } => OpSummary::Replace {
            sheet: number(sheet),
            stroke: stroke.into(),
        },
        CanvasOp::SetImage { sheet, image } => OpSummary::SetImage {
            sheet: number(sheet),
            image: image.into(),
        },
        CanvasOp::RemoveImage { sheet } => OpSummary::RemoveImage {
            sheet: number(sheet),
        },
        CanvasOp::AddSheet { id, .. } => OpSummary::AddSheet { sheet: number(id) },
        // The sheet is gone, so it has no number
        CanvasOp::RemoveSheet { .. } => OpSummary::RemoveSheet,
        CanvasOp::SelectSheet { id } => OpSummary::SelectSheet { sheet: number(id) },
        CanvasOp::ResetSheets => OpSummary::ResetSheets,
    }
}

/// Which revision of the canvas, which sheet, and which area the accompanying image shows
#[derive(Serialize)]
struct ImageInfo {
    revision: u64,
    /// Number of the sheet shown (1-based, in order)
    sheet: usize,
    /// The image's top left is (x, y) on the canvas. width and height are in canvas CSS pixels
    #[serde(flatten)]
    area: Rect,
    /// Image pixels per canvas CSS pixel
    scale: f64,
}

#[derive(Serialize)]
struct CanvasSummary {
    revision: u64,
    /// Number of the sheet in this result (1-based, in order)
    sheet: usize,
    /// Number of the sheet the user is viewing now
    #[serde(rename = "currentSheet")]
    current_sheet: usize,
    /// Number of sheets
    #[serde(rename = "sheetCount")]
    sheet_count: usize,
    /// For each connected canvas page, the paper size (paper starts at (0, 0)) and the visible area
    canvases: Vec<CanvasView>,
    /// No image is returned when there is nothing to draw
    image: Option<ImageInfo>,
    /// The image placed under the strokes. Absent if there is none
    #[serde(rename = "placedImage", skip_serializing_if = "Option::is_none")]
    placed_image: Option<PlacedImage>,
    /// Stacking order (later ones are on top). Absent when asked to omit it
    #[serde(skip_serializing_if = "Option::is_none")]
    strokes: Option<Vec<StrokeSummary>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ChangesSummary {
    Ops {
        revision: u64,
        /// Number of the sheet the user is viewing now
        #[serde(rename = "currentSheet")]
        current_sheet: usize,
        /// Number of sheets
        #[serde(rename = "sheetCount")]
        sheet_count: usize,
        /// Pages connected when called and their visible areas. Changes to the area do not raise the
        /// revision, so they are always returned separately from the diff
        canvases: Vec<CanvasView>,
        ops: Vec<OpSummary>,
        /// No image is returned when nothing changed (the same image as last time is not sent)
        #[serde(skip_serializing_if = "Option::is_none")]
        image: Option<ImageInfo>,
    },
    Snapshot(CanvasSummary),
}

/// The canvas image. Returns (image description, PNG). data is the image of sheet.image
/// (without it, the image is not passed to render, so it is neither drawn nor included in the area. Its
/// placement is given as placedImage in the JSON)
fn render_image(
    snapshot: &Snapshot,
    sheet: &Sheet,
    data: Option<&StoredImage>,
) -> Result<Option<(ImageInfo, Vec<u8>)>, ErrorData> {
    // The whole paper and visible area of connected pages are captured even without strokes
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
    // The image adds to the captured area only where it overlaps the paper of connected pages
    // (outside the paper the user cannot see it either; adding it widens the area and shrinks the handwriting too).
    // If no page is connected or no paper size has arrived, use None and add the whole image area
    let clip = snapshot
        .views
        .iter()
        .filter_map(|view| view.page)
        .map(Rect::from)
        .reduce(Rect::union);
    // Shows that sheet's strokes and image
    let placed = match (&sheet.image, data) {
        (Some(image), Some(stored)) => Some((image, stored)),
        _ => None,
    };
    let image = render::render(&sheet.strokes, &regions, placed, clip).map_err(|error| {
        ErrorData::internal_error(format!("cannot make the image: {error}"), None)
    })?;
    Ok(image.map(|image| {
        let info = ImageInfo {
            revision: snapshot.revision,
            sheet: snapshot
                .sheet_number(&sheet.id)
                .expect("写す紙はスナップショットの中にある"),
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

/// Returns a description of the image placement and the image (PNG) at its original resolution
pub fn placed_image_result(
    image: &ImagePlacement,
    png: &[u8],
) -> Result<CallToolResult, ErrorData> {
    result_with_image(&PlacedImage::from(image), Some(png.to_vec()))
}

/// Returns the description (JSON) and the image if there is one
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
    sheet: &Sheet,
    include_strokes: bool,
    data: Option<&StoredImage>,
) -> Result<(CanvasSummary, Option<Vec<u8>>), ErrorData> {
    let (image, png) = render_image(snapshot, sheet, data)?.unzip();
    let summary = CanvasSummary {
        revision: snapshot.revision,
        sheet: snapshot
            .sheet_number(&sheet.id)
            .expect("読んだ紙はスナップショットの中にある"),
        current_sheet: snapshot
            .sheet_number(&snapshot.current)
            .expect("今の紙は必ず存在する"),
        sheet_count: snapshot.sheets.len(),
        canvases: snapshot.views.clone(),
        image,
        placed_image: sheet.image.as_ref().map(PlacedImage::from),
        strokes: include_strokes.then(|| sheet.strokes.iter().map(StrokeSummary::from).collect()),
    };
    Ok((summary, png))
}

/// Returns the description and image of one sheet. If include_strokes is false, omits the per-stroke list.
/// data is the image of sheet.image
pub fn canvas_result(
    snapshot: &Snapshot,
    sheet: &Sheet,
    include_strokes: bool,
    data: Option<&StoredImage>,
) -> Result<CallToolResult, ErrorData> {
    let (summary, png) = canvas_summary(snapshot, sheet, include_strokes, data)?;
    result_with_image(&summary, png)
}

/// Makes the diff's description and the image of the current sheet. No image if nothing changed.
/// data is the image of the current sheet in current
fn changes_summary(
    changes: &ChangesSince,
    current: &Snapshot,
    data: Option<&StoredImage>,
) -> Result<(ChangesSummary, Option<Vec<u8>>), ErrorData> {
    match changes {
        ChangesSince::Ops { revision, ops } => {
            // When there are changes, the attached image is of the current sheet
            let (image, png) = if ops.is_empty() {
                (None, None)
            } else {
                render_image(current, current.current_sheet(), data)?.unzip()
            };
            let summary = ChangesSummary::Ops {
                revision: *revision,
                current_sheet: current
                    .sheet_number(&current.current)
                    .expect("今の紙は必ず存在する"),
                sheet_count: current.sheets.len(),
                canvases: current.views.clone(),
                ops: ops.iter().map(|op| summarize(op, current)).collect(),
                image,
            };
            Ok((summary, png))
        }
        ChangesSince::Snapshot(snapshot) => {
            // When no diff is available, return the whole canvas (it is the current state, so data can be used as is).
            // It replaces the diff, so the stroke list is not omitted. Returns the current sheet
            let (summary, png) = canvas_summary(snapshot, snapshot.current_sheet(), true, data)?;
            Ok((ChangesSummary::Snapshot(summary), png))
        }
    }
}

/// Returns a description of the changes and the current canvas image. No image if nothing changed.
/// data is the image of current.image
pub fn changes_result(
    changes: &ChangesSince,
    current: &Snapshot,
    data: Option<&StoredImage>,
) -> Result<CallToolResult, ErrorData> {
    let (summary, png) = changes_summary(changes, current, data)?;
    result_with_image(&summary, png)
}

/// The result of wait_for_canvas: whether it was sent (sent) and the page that pressed (canvas) around the description of the changes
#[derive(Serialize)]
struct SentSummary {
    sent: bool,
    /// The number of the page that pressed (the id in canvases)
    canvas: u64,
    #[serde(flatten)]
    changes: ChangesSummary,
}

/// Returns that it was pressed (sent). The description has the same shape as read_canvas_changes,
/// not nested thanks to flatten
pub fn sent_result(
    canvas: u64,
    changes: &ChangesSince,
    current: &Snapshot,
    data: Option<&StoredImage>,
) -> Result<CallToolResult, ErrorData> {
    let (changes, png) = changes_summary(changes, current, data)?;
    let summary = SentSummary {
        sent: true,
        canvas,
        changes,
    };
    result_with_image(&summary, png)
}

/// Returns that it timed out without a press
pub fn not_sent_result() -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![json_block(
        &NotSentSummary { sent: false },
    )?]))
}

/// The result when it timed out without a press (no description of the changes)
#[derive(Serialize)]
struct NotSentSummary {
    sent: bool,
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
    use crate::protocol::{INITIAL_SHEET_ID, PageSize, Sheet, StrokePoint, Viewport};

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

    /// Makes a Snapshot of one sheet (= `INITIAL_SHEET_ID`) from strokes and an image (shared by tests)
    fn snapshot(strokes: Vec<Stroke>, image: Option<ImagePlacement>) -> Snapshot {
        Snapshot {
            revision: 4,
            views: vec![view(2, 30.0, 20.0)],
            sheets: vec![Sheet {
                id: INITIAL_SHEET_ID.into(),
                strokes,
                image,
            }],
            current: INITIAL_SHEET_ID.into(),
        }
    }

    /// A 2x2 solid red PNG
    fn red_png() -> Vec<u8> {
        let rgba: Vec<u8> = (0..4).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
        png
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
        let snapped = snapshot(vec![stroke("a")], None);
        let result = canvas_result(&snapped, snapped.current_sheet(), true, None).unwrap();
        assert_eq!(result.content.len(), 2);
        let image = result.content[1].as_image().unwrap();
        assert_eq!(image.mime_type, "image/png");

        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "revision": 4,
                "sheet": 1,
                "currentSheet": 1,
                "sheetCount": 1,
                "canvases": canvases_json(),
                "image": { "revision": 4, "sheet": 1, "x": 0.0, "y": 0.0, "width": 30.0, "height": 20.0, "scale": 1.0 },
                "strokes": [{
                    "id": "a",
                    "author": "ai",
                    "color": "#000000",
                    "bounds": { "x": 7, "y": 7, "width": 16, "height": 6 }
                }]
            })
        );
    }

    /// With two sheets, reading a sheet that is not current includes only that sheet's strokes,
    /// and sheet is its number
    #[test]
    fn reading_a_sheet_other_than_the_current_one_returns_only_its_strokes() {
        let mut two = snapshot(vec![stroke("a")], None);
        two.sheets.push(Sheet {
            id: "sheet-2".into(),
            strokes: vec![stroke("b")],
            image: None,
        });
        two.current = "sheet-2".into();
        // Read the first sheet (the second is current)
        let result = canvas_result(&two, &two.sheets[0], true, None).unwrap();
        let json = summary_json(&result);
        assert_eq!(json["sheet"], 1);
        assert_eq!(json["currentSheet"], 2);
        assert_eq!(json["sheetCount"], 2);
        // Only the strokes of the sheet read ("b" on the second sheet is not included)
        assert_eq!(
            json["strokes"],
            serde_json::json!([{
                "id": "a",
                "author": "ai",
                "color": "#000000",
                "bounds": { "x": 7, "y": 7, "width": 16, "height": 6 }
            }])
        );
        assert_eq!(json["image"]["sheet"], 1);
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
        let mut both = snapshot(Vec::new(), None);
        both.views = vec![view(1, 30.0, 20.0), view(2, 50.0, 10.0)];
        let json = summary_json(&canvas_result(&both, both.current_sheet(), true, None).unwrap());
        assert_eq!(json["canvases"].as_array().unwrap().len(), 2);
        assert_eq!(json["canvases"][1]["ip"], "192.168.0.2");
        assert_eq!(
            json["image"],
            serde_json::json!({ "revision": 4, "sheet": 1, "x": 0.0, "y": 0.0, "width": 50.0, "height": 20.0, "scale": 1.0 })
        );
    }

    #[test]
    fn strokes_can_be_omitted() {
        let snapped = snapshot(vec![stroke("a")], None);
        let result = canvas_result(&snapped, snapped.current_sheet(), false, None).unwrap();
        assert_eq!(result.content.len(), 2, "画像は返す");
        let json = summary_json(&result);
        assert!(json.get("strokes").is_none());
        assert_eq!(json["revision"], 4);
    }

    #[test]
    fn empty_canvas_has_no_image() {
        let mut empty = snapshot(Vec::new(), None);
        empty.revision = 0;
        empty.views.clear();
        let result = canvas_result(&empty, empty.current_sheet(), true, None).unwrap();
        assert_eq!(result.content.len(), 1);
        assert_eq!(summary_json(&result)["image"], serde_json::Value::Null);
    }

    #[test]
    fn changes_are_summarized() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: vec![
                CanvasOp::Insert {
                    sheet: INITIAL_SHEET_ID.into(),
                    stroke: stroke("a"),
                    index: 0,
                },
                CanvasOp::Remove {
                    sheet: INITIAL_SHEET_ID.into(),
                    ids: vec!["b".into()],
                },
            ],
        };
        let result = changes_result(&changes, &snapshot(vec![stroke("a")], None), None).unwrap();
        let json = summary_json(&result);
        assert_eq!(json["kind"], "ops");
        assert_eq!(json["currentSheet"], 1);
        assert_eq!(json["sheetCount"], 1);
        assert_eq!(json["ops"][0]["type"], "insert");
        assert_eq!(json["ops"][0]["sheet"], 1);
        assert_eq!(json["ops"][0]["stroke"]["id"], "a");
        assert!(json["ops"][0]["stroke"].get("points").is_none());
        assert_eq!(
            json["ops"][1],
            serde_json::json!({ "type": "remove", "sheet": 1, "ids": ["b"] })
        );
        assert_eq!(json["image"]["revision"], 4);
        assert_eq!(json["image"]["sheet"], 1);
        assert_eq!(result.content.len(), 2);
    }

    #[test]
    fn no_changes_return_revision_and_viewport() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: Vec::new(),
        };
        let result = changes_result(&changes, &snapshot(vec![stroke("a")], None), None).unwrap();
        assert_eq!(result.content.len(), 1, "画像は付けない");
        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "kind": "ops",
                "revision": 4,
                "currentSheet": 1,
                "sheetCount": 1,
                "canvases": canvases_json(),
                "ops": []
            })
        );
    }

    /// Diff summaries: stroke changes carry sheet, changes to removed sheets do not.
    /// The shapes of the four sheet changes
    #[test]
    fn ops_carry_their_sheet_number_and_absent_sheets_are_omitted() {
        let mut two = snapshot(vec![stroke("a")], None);
        two.sheets.push(Sheet {
            id: "sheet-2".into(),
            strokes: Vec::new(),
            image: None,
        });
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: vec![
                CanvasOp::Insert {
                    sheet: INITIAL_SHEET_ID.into(),
                    stroke: stroke("a"),
                    index: 0,
                },
                CanvasOp::Remove {
                    sheet: "sheet-2".into(),
                    ids: vec!["b".into()],
                },
                // Changes to removed sheets have no number
                CanvasOp::Remove {
                    sheet: "gone".into(),
                    ids: vec!["c".into()],
                },
                CanvasOp::AddSheet {
                    id: "sheet-2".into(),
                    index: 1,
                },
                CanvasOp::AddSheet {
                    id: "gone".into(),
                    index: 2,
                },
                CanvasOp::RemoveSheet { id: "gone".into() },
                CanvasOp::SelectSheet {
                    id: "sheet-2".into(),
                },
                CanvasOp::SelectSheet { id: "gone".into() },
                CanvasOp::ResetSheets,
            ],
        };
        let json = summary_json(&changes_result(&changes, &two, None).unwrap());
        assert_eq!(json["currentSheet"], 1);
        assert_eq!(json["sheetCount"], 2);
        assert_eq!(
            json["ops"][0],
            serde_json::json!({
                "type": "insert",
                "sheet": 1,
                "index": 0,
                "stroke": {
                    "id": "a",
                    "author": "ai",
                    "color": "#000000",
                    "bounds": { "x": 7, "y": 7, "width": 16, "height": 6 }
                }
            })
        );
        assert_eq!(
            json["ops"][1],
            serde_json::json!({ "type": "remove", "sheet": 2, "ids": ["b"] })
        );
        assert_eq!(
            json["ops"][2],
            serde_json::json!({ "type": "remove", "ids": ["c"] })
        );
        assert_eq!(
            json["ops"][3],
            serde_json::json!({ "type": "addSheet", "sheet": 2 })
        );
        assert_eq!(json["ops"][4], serde_json::json!({ "type": "addSheet" }));
        assert_eq!(json["ops"][5], serde_json::json!({ "type": "removeSheet" }));
        assert_eq!(
            json["ops"][6],
            serde_json::json!({ "type": "selectSheet", "sheet": 2 })
        );
        assert_eq!(json["ops"][7], serde_json::json!({ "type": "selectSheet" }));
        assert_eq!(json["ops"][8], serde_json::json!({ "type": "resetSheets" }));
    }

    #[test]
    fn not_sent_is_only_sent_false() {
        let result = not_sent_result().unwrap();
        assert_eq!(result.content.len(), 1, "画像は無い");
        assert_eq!(summary_json(&result), serde_json::json!({ "sent": false }));
    }

    #[test]
    fn sent_is_flattened_with_the_changes_and_has_image() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: vec![CanvasOp::Insert {
                sheet: INITIAL_SHEET_ID.into(),
                stroke: stroke("a"),
                index: 0,
            }],
        };
        let result = sent_result(7, &changes, &snapshot(vec![stroke("a")], None), None).unwrap();
        assert_eq!(result.content.len(), 2, "変更があれば画像付き");
        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "sent": true,
                "canvas": 7,
                "kind": "ops",
                "revision": 4,
                "currentSheet": 1,
                "sheetCount": 1,
                "canvases": canvases_json(),
                "ops": [{
                    "type": "insert",
                    "sheet": 1,
                    "index": 0,
                    "stroke": {
                        "id": "a",
                        "author": "ai",
                        "color": "#000000",
                        "bounds": { "x": 7, "y": 7, "width": 16, "height": 6 }
                    }
                }],
                "image": { "revision": 4, "sheet": 1, "x": 0.0, "y": 0.0, "width": 30.0, "height": 20.0, "scale": 1.0 }
            })
        );
        // Not nested thanks to flatten
        assert!(summary_json(&result).get("changes").is_none());
    }

    #[test]
    fn sent_without_changes_has_no_image() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: Vec::new(),
        };
        let result = sent_result(7, &changes, &snapshot(vec![stroke("a")], None), None).unwrap();
        assert_eq!(result.content.len(), 1, "変更が無ければ画像なし");
        assert_eq!(
            summary_json(&result),
            serde_json::json!({
                "sent": true,
                "canvas": 7,
                "kind": "ops",
                "revision": 4,
                "currentSheet": 1,
                "sheetCount": 1,
                "canvases": canvases_json(),
                "ops": []
            })
        );
    }

    fn placement(id: &str, x: f64, y: f64, width: f64, height: f64) -> ImagePlacement {
        ImagePlacement {
            id: id.into(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn placed_image_is_described_and_drawn_under_strokes() {
        // An image that fully covers the strokes (7..23, 7..13)
        let mut snapped = snapshot(
            vec![stroke("a")],
            Some(placement("img", 5.0, 5.0, 30.0, 20.0)),
        );
        snapped.views.clear();
        let result = canvas_result(
            &snapped,
            snapped.current_sheet(),
            true,
            Some(&StoredImage::new(red_png())),
        )
        .unwrap();
        let json = summary_json(&result);
        assert_eq!(
            json["placedImage"],
            serde_json::json!({ "id": "img", "bounds": { "x": 5, "y": 5, "width": 30, "height": 20 } })
        );
        // The image area is captured too
        assert_eq!(json["image"]["x"], 5.0);
        assert_eq!(json["image"]["width"], 30.0);
        assert_eq!(result.content.len(), 2, "合成した PNG も返す");
    }

    /// Even if the image is placed larger than the paper, the captured area stays the paper (the handwriting does not shrink)
    #[test]
    fn image_beyond_the_page_does_not_grow_the_rendered_area() {
        // An image that extends far past the right and bottom of the paper (30×20)
        let snapped = snapshot(
            vec![stroke("a")],
            Some(placement("img", 10.0, 10.0, 4000.0, 3000.0)),
        );
        let result = canvas_result(
            &snapped,
            snapped.current_sheet(),
            true,
            Some(&StoredImage::new(red_png())),
        )
        .unwrap();
        let json = summary_json(&result);
        assert_eq!(
            json["image"],
            serde_json::json!({ "revision": 4, "sheet": 1, "x": 0.0, "y": 0.0, "width": 30.0, "height": 20.0, "scale": 1.0 })
        );
        // The area the image covers is returned as the placement itself
        assert_eq!(
            json["placedImage"],
            serde_json::json!({ "id": "img", "bounds": { "x": 10, "y": 10, "width": 4000, "height": 3000 } })
        );
    }

    /// When no page is connected the paper is unknown, so the whole image area is captured
    #[test]
    fn image_area_is_whole_without_any_page() {
        let mut snapped = snapshot(
            Vec::new(),
            Some(placement("img", 10.0, 10.0, 4000.0, 3000.0)),
        );
        snapped.views.clear();
        let json = summary_json(
            &canvas_result(
                &snapped,
                snapped.current_sheet(),
                true,
                Some(&StoredImage::new(red_png())),
            )
            .unwrap(),
        );
        assert_eq!(json["image"]["x"], 10.0);
        assert_eq!(json["image"]["width"], 4000.0);
        assert!(
            json["image"]["scale"].as_f64().unwrap() < 1.0,
            "広い範囲は縮む"
        );
    }

    #[test]
    fn image_ops_are_summarized() {
        let changes = ChangesSince::Ops {
            revision: 4,
            ops: vec![
                CanvasOp::SetImage {
                    sheet: INITIAL_SHEET_ID.into(),
                    image: placement("img", 1.0, 2.0, 10.5, 4.0),
                },
                CanvasOp::RemoveImage {
                    sheet: INITIAL_SHEET_ID.into(),
                },
            ],
        };
        let json = summary_json(
            &changes_result(&changes, &snapshot(vec![stroke("a")], None), None).unwrap(),
        );
        assert_eq!(
            json["ops"][0],
            serde_json::json!({
                "type": "setImage",
                "sheet": 1,
                "image": { "id": "img", "bounds": { "x": 1, "y": 2, "width": 11, "height": 4 } }
            })
        );
        assert_eq!(
            json["ops"][1],
            serde_json::json!({ "type": "removeImage", "sheet": 1 })
        );
    }
}
