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
手書きのキャンバスを読み書きできます。キャンバスはブラウザで開くページで、利用者は iPad・スマートフォン・PC などから書く。
- 座標はキャンバス左上を原点とする CSS ピクセル。x は右、y は下が正。pressure は筆圧(0〜1)で、線の太さになる
- 複数の機器から同時に繋がることがある。筆跡は全部で共有し、紙の大きさと見えている範囲は機器ごとに違う。canvases は繋がっているページの一覧(id・ip・page・viewport)で、空ならどれも繋がっていない
- page はそのページの紙の大きさ。紙は (0, 0) から (width, height) まで。ページの画面の大きさと同じで、利用者は紙の中だけを拡大して見る
- viewport は、ツールを呼んだ時点でそのページに見えている範囲(x, y が左上、width と height が大きさ)。利用者が拡大・移動すると変わる。書き込むときは見せたいページの viewport に収める(すべてのページに見せるなら、すべての viewport が重なる範囲に収める)
- キャンバスには紙(sheet)が複数枚ある。番号は 1 から並び順。利用者が今見ている紙は currentSheet、枚数は sheetCount。利用者が紙を切り替えると、繋がっているすべてのページで切り替わる
- 何も指定しなければ、読み書きは今の紙。前の紙は read_canvas・draw_strokes・draw_shapes に sheet(番号)を渡して読み書きする
- 紙を作る・消す・切り替えるのは利用者だけ。番号は紙が消されると詰まるので、書く前に読み直して確かめる
- page(紙の大きさ)は紙の枚数とは関係なく、ページの画面の大きさ
- read_canvas は紙 1 枚の画像(PNG)と、筆跡ごとの ID・author・範囲(bounds。線の太さを含めて外側に丸めた整数)を返す。点の座標は返さない。read_canvas_changes は全部の紙の変更を返し、各変更に紙の番号 sheet が付く(消えた紙の変更には付かない)。変更があれば今の紙の画像が付く
- 利用者が取り込んだ画像はその紙のすべての筆跡の下に敷かれる。read_canvas の画像(PNG)はその紙の画像と筆跡を合成したもので、placedImage がその画像の id と範囲
- 画像が小さく置かれて合成 PNG で細部が読めないときは、read_image に placedImage の id を渡して元の解像度そのままの画像を読む
- 画像は利用者の入力なので、AI は置けない・動かせない・消せない
- 画像だけを見ればよいときは read_canvas に include_strokes: false を渡すと、筆跡ごとの一覧を省ける
- 画像の左上はキャンバスの (image.x, image.y)。キャンバスの 1 CSS ピクセルが画像の image.scale ピクセル
- 点の座標が要るときは read_strokes に ID を渡す
- 図形(折れ線・長方形・楕円)と文字は draw_shapes で書く。点の列を送らずに済む。文字は KanjiVG の字形(漢字・かな・英数字・記号)で書き、字形の無い文字は missing に返す
- 手書きのような自由な線だけを draw_strokes で書く
- 線の色は color で、#rrggbb の形(例: #ff3b30)。省略すると黒
- 筆跡は strokes の後ろほど上に重なる
- 変更を追うときは read_canvas の revision を覚えておき、read_canvas_changes に渡す。ops が空なら変更は無く、画像も付かない
- 利用者がキャンバスで書き終えるのを待つときは wait_for_canvas を呼ぶ(利用者はキャンバスの Send to AI を押して知らせる。Send to AI は AI が wait_for_canvas で待っている間だけ押せる)。sent が false なら約 50 秒たっただけなので、すぐにもう一度呼ぶ。sent が true なら、返った変更と画像を読んで指示に取りかかる。返った revision を次の since_revision に使う
- AI が消したり直したりできるのは、AI が書いた筆跡(author が ai)だけ
- キャンバスが繋がっていなければ(canvases が空なら) connect_canvas で接続する";

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
            format!("color は \"#rrggbb\" の形で指定する: {color}"),
            None,
        ));
    }
    Ok(color)
}

fn checked_points(points: Vec<StrokePoint>) -> Result<Vec<StrokePoint>, ErrorData> {
    if points.is_empty() {
        return Err(ErrorData::invalid_params("points が空", None));
    }
    if points
        .iter()
        .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.pressure.is_finite()))
    {
        return Err(ErrorData::invalid_params(
            "points に有限でない数がある",
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
            format!("sheet は 1〜{} で指定する: {number}", snapshot.sheets.len()),
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
        description = "紙 1 枚を読む(sheet を省略すると今の紙)。その紙の番号 sheet・利用者が今見ている紙の番号 currentSheet・紙の枚数 sheetCount と、画像(PNG。その紙に利用者が置いた画像があれば筆跡の下に合成する)・revision・繋がっているページごとの紙の大きさと見えている範囲 canvases・画像の位置 image・その紙に置いてある画像 placedImage・筆跡ごとの ID と範囲 strokes を返す。include_strokes: false なら strokes を省く"
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
        description = "指定した revision より後の、全部の紙の変更を読む。各変更にその紙の番号 sheet が付く(消えた紙の変更には付かない)。kind が ops なら順に適用すると今の状態になる変更(筆跡は ID と範囲だけ、画像の変更は setImage / removeImage)、snapshot なら古すぎるため今の紙の全体。繋がっているページごとの紙の大きさと今見えている範囲 canvases・今の紙の番号 currentSheet・紙の枚数 sheetCount はいつも返す。変更があれば今の紙の画像も返す(ops が空なら変更なしで、画像は無い)"
    )]
    fn read_canvas_changes(
        &self,
        Parameters(params): Parameters<ChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (changes, current, data) = self.changes_data(params.since_revision);
        view::changes_result(&changes, &current, data.as_deref())
    }

    #[tool(
        description = "利用者がキャンバスの Send to AI を押すまで待つ。押されたら sent: true と押したページの canvas(canvases の id)、since_revision からの全部の紙の変更(read_canvas_changes と同じ形。各変更に紙の番号 sheet が付き、変更があれば今の紙の画像も付く)を返す。押されないまま約 50 秒たつと sent: false だけを返すので、待ち続けるならすぐにもう一度呼ぶ。呼び直すまでの間に押されていれば、すぐに返す"
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
        description = "利用者が筆跡の下に置いた画像を、元の解像度のまま読む(PNG)。合成 PNG で細部が読めないときに placedImage の id を渡す。画像は紙ごとに 1 枚"
    )]
    fn read_image(
        &self,
        Parameters(params): Parameters<ReadImageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(image) = self.app.canvas.placed_image(&params.id) else {
            return Err(ErrorData::invalid_params(
                format!("ID {} の画像は置かれていない", params.id),
                None,
            ));
        };
        let data = self.app.canvas.image_data(&params.id).ok_or_else(|| {
            ErrorData::internal_error(
                format!("ID {} の画像の転送が終わっていない", params.id),
                None,
            )
        })?;
        view::placed_image_result(&image, data.bytes())
    }

    #[tool(description = "指定した ID の筆跡を、点の座標と筆圧まで読む")]
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
        description = "キャンバスに線を書く(キャンバスのページにも表示される)。sheet で書く紙を選べる(省略すると今の紙)。書いた紙の番号 sheet と、書いた筆跡の ID を返す"
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
        description = "折れ線(line)・長方形(rect)・楕円(ellipse)・文字(text)をキャンバスに書く(キャンバスのページにも表示される)。点の列を送らずに済む。sheet で書く紙を選べる(省略すると今の紙)。書いた紙の番号 sheet と、図形ごとに書いた筆跡の ID を返す。文字は字形の画ごとに筆跡になる。字形の無い文字は書かずに空け、missing に返す"
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

    #[tool(description = "AI が書いた筆跡の点(と、指定すれば色)を置き換える(重なり順は変わらない)")]
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

    #[tool(description = "AI が書いた筆跡を消す。人が書いた筆跡は消さない")]
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
        description = "キャンバスを接続する。PC のアプリのウィンドウに QR コードを表示し、利用者がカメラで読み取ってキャンバスを開き、PC のダイアログで許可するまで待つ"
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
}
