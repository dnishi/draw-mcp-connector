//! MCP server through which AI agents read and write the canvas (Streamable HTTP, 127.0.0.1 only).

mod connect;
mod view;

use std::net::Ipv4Addr;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::app::{App, BoxError};
use crate::canvas::{ChangesSince, Origin, Snapshot};
use crate::color;
use crate::config;
use crate::net;
use crate::protocol::{Author, CanvasOp, Sheet, Stroke, StrokePoint};
use crate::render::StoredImage;
use crate::shapes::{self, Shape};

const INSTRUCTIONS: &str = "\
You can read and write on a handwritten canvas. The canvas is a page opened in a browser, and the user writes from a device such as an iPad, a smartphone, or a PC.
- Coordinates are CSS pixels with the origin at the top left of the canvas. x is positive to the right, and y is positive down. pressure is the pen pressure (0 to 1), which sets the line width
- Multiple devices may be connected at the same time. Strokes are shared across all, and the size of the sheet and the visible range differ per device. canvases is a list of the connected pages (id, ip, page, viewport), and if it is empty, none is connected
- page is the size of the sheet of that page. The sheet is from (0, 0) to (width, height). It is the same size as the screen of the page, and the user only zooms inside the sheet
- viewport is the range visible on that page at the time the tool is called (x, y are the top left, and width and height are the size). It changes when the user zooms or moves. When writing, keep what you write inside the viewport of the page you want to show it on (to show it on every page, keep it inside the range where all viewports overlap)
- The canvas has multiple sheets (sheet). The numbers are 1-based, in order. The sheet the user is currently viewing is currentSheet, and the number of sheets is sheetCount. When the user switches sheets, every connected page switches
- If nothing is specified, the tool reads and writes the current sheet. To read or write a previous sheet, pass sheet (a number) to read_canvas, draw_strokes, or draw_shapes
- Only the user creates, deletes, and switches sheets. Numbers shift when a sheet is deleted, so read again to confirm before writing
- page (the size of the sheet) is the size of the screen of the page, regardless of the number of sheets
- read_canvas returns the image (PNG) of one sheet and, for each stroke, its ID, author, and bounds (rounded out to an integer, including the line width). It does not return point coordinates. read_canvas_changes returns the changes of all sheets, and each change has the sheet number sheet (a change of a deleted sheet has none). If there are changes, the image of the current sheet is attached
- The image the user placed is under all the strokes of its sheet. The image (PNG) of read_canvas is the composited PNG of the sheet image and the strokes, and placedImage is the id and the range of that image
- When the image is placed small and the details cannot be read in the composited PNG, pass the id of placedImage to read_image to read the image at the original resolution
- The placed image is the user's input, so the AI cannot place, move, or remove it
- When only the image is needed, pass include_strokes: false to read_canvas to omit the per-stroke list
- The top left of the image is at (image.x, image.y) of the canvas. One CSS pixel of the canvas is image.scale pixels of the image
- When point coordinates are needed, pass the IDs to read_strokes
- Shapes (polyline, rectangle, ellipse) and text are written with draw_shapes, so there is no need to send a list of points. Text is written with the KanjiVG glyphs (kanji, kana, alphanumerics, symbols), and characters without a glyph are returned in missing
- Use draw_strokes only for freehand lines like handwriting
- The line color is color, in the form #rrggbb (example: #ff3b30). Black if omitted
- Strokes are stacked, and later ones in strokes are on top
- To follow changes, remember the revision of read_canvas and pass it to read_canvas_changes. If ops is empty, there are no changes and no image
- To wait until the user finishes writing on the canvas, call wait_for_canvas (the user presses Send to AI on the canvas to signal. Send to AI can be pressed only while the AI is waiting with wait_for_canvas). If sent is false, only about 50 seconds have passed, so call it again right away. If sent is true, read the returned changes and image and start working on the instructions. Use the returned revision as the next since_revision
- The AI can remove or modify only the strokes written by the AI (author is ai)
- If the canvas is not connected (if canvases is empty), connect it with connect_canvas";

// The /// doc comments are sent to the AI as tool parameter descriptions, so they are written in English
#[derive(Deserialize, JsonSchema)]
struct ReadCanvasParams {
    /// Number of the sheet to read (1-based, in order). Omit for the current sheet
    sheet: Option<usize>,
    /// If false, omits the per-stroke list (strokes) and returns only the image, revision, etc. Defaults to true
    include_strokes: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct ChangesParams {
    /// The revision received last time
    since_revision: u64,
}

#[derive(Deserialize, JsonSchema)]
struct ReadStrokesParams {
    /// IDs of the strokes to read
    ids: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ReadImageParams {
    /// ID of the image to read (placedImage.id of read_canvas)
    id: String,
}

#[derive(Serialize)]
struct ReadStrokesResult {
    strokes: Vec<Stroke>,
    /// IDs that were not found
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct NewStroke {
    /// Points the line passes through (2 or more make a line)
    points: Vec<StrokePoint>,
    /// Line color ("#rrggbb"). Black if omitted
    color: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DrawParams {
    strokes: Vec<NewStroke>,
    /// Number of the sheet to draw on (1-based, in order). Omit for the current sheet
    sheet: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct DrawShapesParams {
    /// Shapes to write. Later ones are stacked on top
    shapes: Vec<Shape>,
    /// Number of the sheet to draw on (1-based, in order). Omit for the current sheet
    sheet: Option<usize>,
}

#[derive(Serialize)]
struct DrawShapesResult {
    revision: u64,
    /// Number of the sheet drawn on
    sheet: usize,
    /// IDs of the strokes drawn for each shape (in the same order as shapes)
    ids: Vec<Vec<String>>,
    /// Characters that could not be written for lack of a glyph
    #[serde(skip_serializing_if = "String::is_empty")]
    missing: String,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateParams {
    /// ID of the stroke to modify (written by the AI)
    id: String,
    /// New list of points
    points: Vec<StrokePoint>,
    /// New color ("#rrggbb"). Keeps the current color if omitted
    color: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteParams {
    /// IDs of the strokes to remove (written by the AI)
    ids: Vec<String>,
}

#[derive(Serialize)]
struct WriteResult {
    revision: u64,
    /// Number of the sheet drawn on (only when drawing new strokes; omitted by update_ai_stroke and delete_ai_strokes)
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ids: Vec<String>,
    /// IDs not handled because they were not written by the AI or do not exist
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped: Vec<String>,
}

#[derive(Clone)]
pub struct CanvasServer {
    app: Arc<App>,
    tool_router: ToolRouter<Self>,
}

fn json_result(value: &impl Serialize) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![view::json_block(value)?]))
}

/// Validates a color. fallback if omitted
fn checked_color(color: Option<String>, fallback: &str) -> Result<String, ErrorData> {
    let color = color.unwrap_or_else(|| fallback.into());
    if color::parse(&color).is_none() {
        return Err(ErrorData::invalid_params(
            format!("color must be in the form \"#rrggbb\": {color}"),
            None,
        ));
    }
    Ok(color)
}

fn checked_points(points: Vec<StrokePoint>) -> Result<Vec<StrokePoint>, ErrorData> {
    if points.is_empty() {
        return Err(ErrorData::invalid_params("points is empty", None));
    }
    if points
        .iter()
        .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.pressure.is_finite()))
    {
        return Err(ErrorData::invalid_params(
            "points has a non-finite number",
            None,
        ));
    }
    Ok(points
        .into_iter()
        .map(|p| StrokePoint {
            pressure: p.pressure.clamp(0.0, 1.0),
            ..p
        })
        .collect())
}

/// Returns the sheet with that number (1-based, in order) and its number. Omit for the current sheet.
/// Out of range (0, or more than the number of sheets) is an error
fn sheet_by_number(
    snapshot: &Snapshot,
    sheet: Option<usize>,
) -> Result<(usize, &Sheet), ErrorData> {
    let number = sheet.unwrap_or_else(|| {
        snapshot
            .sheet_number(&snapshot.current)
            .expect("今の紙は必ず存在する")
    });
    let Some(sheet) = number
        .checked_sub(1)
        .and_then(|index| snapshot.sheets.get(index))
    else {
        return Err(ErrorData::invalid_params(
            format!("sheet must be 1 to {}: {number}", snapshot.sheets.len()),
            None,
        ));
    };
    Ok((number, sheet))
}

#[tool_router]
impl CanvasServer {
    pub fn new(app: Arc<App>) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }

    fn revision(&self) -> u64 {
        self.app.canvas.snapshot().revision
    }

    /// Stacks the strokes on top as AI strokes and returns their IDs. Takes the ID of the sheet to draw on
    fn insert_ai_strokes(
        &self,
        sheet: &str,
        strokes: Vec<(String, Vec<StrokePoint>)>,
    ) -> Vec<String> {
        strokes
            .into_iter()
            .map(|(color, points)| {
                let id = format!("ai-{}", net::random_hex(8));
                let stroke = Stroke {
                    id: id.clone(),
                    author: Author::Ai,
                    color,
                    points,
                };
                let index = usize::MAX; // stack on top
                self.app.canvas.apply(
                    Origin::Ai,
                    CanvasOp::Insert {
                        sheet: sheet.into(),
                        stroke,
                        index,
                    },
                );
                id
            })
            .collect()
    }

    /// Picks out only the strokes written by the AI
    fn split_ai_owned(&self, ids: Vec<String>) -> (Vec<String>, Vec<String>) {
        ids.into_iter().partition(|id| {
            self.app
                .canvas
                .find_stroke(id)
                .is_some_and(|(_, stroke)| stroke.author == Author::Ai)
        })
    }

    /// Groups stroke IDs by the sheet they are on (dropping missing IDs, keeping the order of the strokes)
    fn group_by_sheet(&self, ids: &[String]) -> Vec<(String, Vec<String>)> {
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        for id in ids {
            let Some((sheet, _)) = self.app.canvas.find_stroke(id) else {
                continue;
            };
            match grouped
                .iter_mut()
                .find(|(group_sheet, _)| *group_sheet == sheet)
            {
                Some(group) => group.1.push(id.clone()),
                None => grouped.push((sheet, vec![id.clone()])),
            }
        }
        grouped
    }

    /// Data of the placed image (on that sheet). None before it arrives or after it was dropped
    fn placed_image_data(&self, sheet: &Sheet) -> Option<std::sync::Arc<StoredImage>> {
        sheet
            .image
            .as_ref()
            .and_then(|image| self.app.canvas.image_data(&image.id))
    }

    /// Takes the changes since since_revision, the current snapshot, and the image data of the current sheet.
    /// The snapshot is taken after the diff, so the image is from the same or a newer revision than the diff
    /// (shown by image.revision)
    fn changes_data(
        &self,
        since_revision: u64,
    ) -> (ChangesSince, Snapshot, Option<std::sync::Arc<StoredImage>>) {
        let changes = self.app.canvas.changes_since(since_revision);
        let current = self.app.canvas.snapshot();
        // The image attached to a diff is of the current sheet
        let data = self.placed_image_data(current.current_sheet());
        (changes, current, data)
    }

    #[tool(
        description = "Reads one sheet (omit sheet for the current sheet). Returns sheet (the number of that sheet), currentSheet (the number of the sheet the user is currently viewing), sheetCount (the number of sheets), the image (PNG; if the user placed an image on that sheet, it is composited under the strokes), revision, canvases (the sheet size and the visible range per connected page), image (the position of the image), placedImage (the image placed on that sheet), and strokes (the ID and bounds of each stroke). With include_strokes: false, omits strokes"
    )]
    fn read_canvas(
        &self,
        Parameters(params): Parameters<ReadCanvasParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let include_strokes = params.include_strokes.unwrap_or(true);
        let snapshot = self.app.canvas.snapshot();
        let (_, sheet) = sheet_by_number(&snapshot, params.sheet)?;
        let data = self.placed_image_data(sheet);
        view::canvas_result(&snapshot, sheet, include_strokes, data.as_deref())
    }

    #[tool(
        description = "Reads the changes of all sheets after the given revision. Each change has sheet (the number of its sheet; a change of a deleted sheet has none). If kind is ops, the result is the changes that reach the current state when applied in order (a stroke has only its ID and bounds, and an image change is setImage / removeImage); if kind is snapshot, the revision is too old, so the result is the whole current sheet. Always returns canvases (the sheet size and the currently visible range per connected page), currentSheet (the number of the current sheet), and sheetCount (the number of sheets). If there are changes, also returns the image of the current sheet (if ops is empty, there are no changes and no image)"
    )]
    fn read_canvas_changes(
        &self,
        Parameters(params): Parameters<ChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (changes, current, data) = self.changes_data(params.since_revision);
        view::changes_result(&changes, &current, data.as_deref())
    }

    #[tool(
        description = "Waits until the user presses Send to AI on the canvas. When pressed, returns sent: true, canvas (the page that pressed it; its id in canvases), and the changes of all sheets since since_revision (same shape as read_canvas_changes; each change has sheet, the number of its sheet, and if there are changes, the image of the current sheet is attached). If about 50 seconds pass without being pressed, returns only sent: false, so to keep waiting, call it again right away. If it was pressed before you call again, returns at once"
    )]
    async fn wait_for_canvas(
        &self,
        Parameters(params): Parameters<ChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Waits for a press of the canvas's Send to AI (long polling).
        // Shorter than Codex's default MCP tool timeout (60 seconds), so it does not hit that limit
        let Some(canvas) = self
            .app
            .ai_inbox
            .wait(config::WAIT_FOR_CANVAS_TIMEOUT)
            .await
        else {
            return view::not_sent_result();
        };
        let (changes, current, data) = self.changes_data(params.since_revision);
        view::sent_result(canvas, &changes, &current, data.as_deref())
    }

    #[tool(
        description = "Reads the image the user placed under the strokes at the original resolution (PNG). Pass the id of placedImage when the details cannot be read in the composited PNG. There is one image per sheet"
    )]
    fn read_image(
        &self,
        Parameters(params): Parameters<ReadImageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(image) = self.app.canvas.placed_image(&params.id) else {
            return Err(ErrorData::invalid_params(
                format!("the image with ID {} is not placed", params.id),
                None,
            ));
        };
        let data = self.app.canvas.image_data(&params.id).ok_or_else(|| {
            ErrorData::internal_error(
                format!(
                    "the transfer of the image with ID {} has not finished",
                    params.id
                ),
                None,
            )
        })?;
        view::placed_image_result(&image, data.bytes())
    }

    #[tool(
        description = "Reads the strokes with the given IDs, down to the point coordinates and pressure"
    )]
    fn read_strokes(
        &self,
        Parameters(params): Parameters<ReadStrokesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut result = ReadStrokesResult {
            strokes: Vec::new(),
            missing: Vec::new(),
        };
        for id in params.ids {
            match self.app.canvas.find_stroke(&id) {
                Some((_, stroke)) => result.strokes.push(stroke),
                None => result.missing.push(id),
            }
        }
        json_result(&result)
    }

    #[tool(
        description = "Writes lines on the canvas (shown on the canvas page too). Pick the sheet to write on with sheet (omit for the current sheet). Returns sheet (the number of the sheet written on) and the IDs of the strokes written"
    )]
    fn draw_strokes(
        &self,
        Parameters(params): Parameters<DrawParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Validate everything before writing (so a failure halfway does not leave a partial write)
        let strokes = params
            .strokes
            .into_iter()
            .map(|stroke| {
                Ok((
                    checked_color(stroke.color, color::BLACK)?,
                    checked_points(stroke.points)?,
                ))
            })
            .collect::<Result<Vec<_>, ErrorData>>()?;
        // Look up the sheet to draw on once, before calling insert
        let snapshot = self.app.canvas.snapshot();
        let (number, sheet) = sheet_by_number(&snapshot, params.sheet)?;
        let ids = self.insert_ai_strokes(&sheet.id, strokes);
        json_result(&WriteResult {
            revision: self.revision(),
            sheet: Some(number),
            ids,
            skipped: Vec::new(),
        })
    }

    #[tool(
        description = "Writes a polyline (line), a rectangle (rect), an ellipse (ellipse), or text (text) on the canvas (shown on the canvas page too). There is no need to send a list of points. Pick the sheet to write on with sheet (omit for the current sheet). Returns sheet (the number of the sheet written on) and the IDs of the strokes written for each shape. Text becomes one stroke per stroke of its glyphs. Characters without a glyph are left blank and returned in missing"
    )]
    fn draw_shapes(
        &self,
        Parameters(params): Parameters<DrawShapesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Validate everything before writing (so a failure halfway does not leave a partial write)
        let mut missing = String::new();
        let mut groups = Vec::with_capacity(params.shapes.len());
        for (index, shape) in params.shapes.into_iter().enumerate() {
            let outline = shapes::outline(&shape).map_err(|message| {
                ErrorData::invalid_params(format!("shapes[{index}]: {message}"), None)
            })?;
            let color = checked_color(shape.color, color::BLACK)?;
            for c in outline.missing {
                if !missing.contains(c) {
                    missing.push(c);
                }
            }
            groups.push(
                outline
                    .strokes
                    .into_iter()
                    .map(|points| (color.clone(), points))
                    .collect::<Vec<_>>(),
            );
        }
        // Look up the sheet to draw on once, before calling insert
        let snapshot = self.app.canvas.snapshot();
        let (number, sheet) = sheet_by_number(&snapshot, params.sheet)?;
        let ids = groups
            .into_iter()
            .map(|strokes| self.insert_ai_strokes(&sheet.id, strokes))
            .collect();
        json_result(&DrawShapesResult {
            revision: self.revision(),
            sheet: number,
            ids,
            missing,
        })
    }

    #[tool(
        description = "Replaces the points (and the color if given) of a stroke written by the AI (the stacking order does not change)"
    )]
    fn update_ai_stroke(
        &self,
        Parameters(params): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (owned, skipped) = self.split_ai_owned(vec![params.id]);
        if let Some((sheet, current)) = owned.first().and_then(|id| self.app.canvas.find_stroke(id))
        {
            let stroke = Stroke {
                color: checked_color(params.color, &current.color)?,
                points: checked_points(params.points)?,
                ..current
            };
            self.app
                .canvas
                .apply(Origin::Ai, CanvasOp::Replace { sheet, stroke });
        }
        json_result(&WriteResult {
            revision: self.revision(),
            sheet: None,
            ids: owned,
            skipped,
        })
    }

    #[tool(
        description = "Removes the strokes written by the AI. Does not remove the strokes written by a human"
    )]
    fn delete_ai_strokes(
        &self,
        Parameters(params): Parameters<DeleteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (owned, skipped) = self.split_ai_owned(params.ids);
        for (sheet, ids) in self.group_by_sheet(&owned) {
            self.app
                .canvas
                .apply(Origin::Ai, CanvasOp::Remove { sheet, ids });
        }
        json_result(&WriteResult {
            revision: self.revision(),
            sheet: None,
            ids: owned,
            skipped,
        })
    }

    #[tool(
        description = "Connects the canvas. Shows a QR code in the window of the PC app, and waits until the user scans it with a camera, opens the canvas, and approves it in the dialog on the PC"
    )]
    async fn connect_canvas(&self) -> Result<CallToolResult, ErrorData> {
        let text = connect::connect_canvas(&self.app).await;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }
}

// By default the list is rebuilt on every tool call, so reuse one built in advance
#[tool_handler(router = self.tool_router)]
impl ServerHandler for CanvasServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                config::MCP_SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

pub fn url() -> String {
    format!("http://127.0.0.1:{}{}", config::MCP_PORT, config::MCP_PATH)
}

pub async fn run(app: Arc<App>) -> Result<(), BoxError> {
    let service = StreamableHttpService::new(
        move || Ok(CanvasServer::new(app.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_allowed_hosts(config::MCP_ALLOWED_HOSTS),
    );
    let router = axum::Router::new().nest_service(config::MCP_PATH, service);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, config::MCP_PORT)).await?;
    info!("MCP: {} で待ち受け", url());
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{INITIAL_SHEET_ID, Sheet};

    /// A snapshot with two sheets (the second is current)
    fn two_sheets() -> Snapshot {
        Snapshot {
            revision: 1,
            views: Vec::new(),
            sheets: vec![
                Sheet {
                    id: INITIAL_SHEET_ID.into(),
                    strokes: Vec::new(),
                    image: None,
                },
                Sheet {
                    id: "sheet-2".into(),
                    strokes: Vec::new(),
                    image: None,
                },
            ],
            current: "sheet-2".into(),
        }
    }

    /// Omitted means the current sheet
    #[test]
    fn sheet_by_number_defaults_to_the_current_sheet() {
        let snapshot = two_sheets();
        let (number, sheet) = sheet_by_number(&snapshot, None).unwrap();
        assert_eq!(number, 2);
        assert_eq!(sheet.id, "sheet-2");
    }

    /// A number in range is the sheet at that position
    #[test]
    fn sheet_by_number_resolves_a_sheet_in_range() {
        let snapshot = two_sheets();
        let (number, sheet) = sheet_by_number(&snapshot, Some(1)).unwrap();
        assert_eq!(number, 1);
        assert_eq!(sheet.id, INITIAL_SHEET_ID);
    }

    /// 0 and numbers beyond the number of sheets are errors
    #[test]
    fn sheet_by_number_rejects_out_of_range() {
        let snapshot = two_sheets();
        assert!(sheet_by_number(&snapshot, Some(0)).is_err());
        assert!(sheet_by_number(&snapshot, Some(3)).is_err());
    }

    /// INSTRUCTIONS sent to the AI are ASCII only
    #[test]
    fn instructions_are_ascii() {
        assert!(
            INSTRUCTIONS.is_ascii(),
            "INSTRUCTIONS has a non-ASCII character"
        );
    }

    /// Tool descriptions and the JSON Schema descriptions of their parameters are ASCII only
    #[test]
    fn tool_descriptions_are_ascii() {
        let non_ascii: Vec<String> = CanvasServer::tool_router()
            .list_all()
            .iter()
            .filter(|tool| !serde_json::to_string(tool).unwrap().is_ascii())
            .map(|tool| tool.name.to_string())
            .collect();
        assert!(
            non_ascii.is_empty(),
            "ASCII でない文字があるツール: {non_ascii:?}"
        );
    }
}
