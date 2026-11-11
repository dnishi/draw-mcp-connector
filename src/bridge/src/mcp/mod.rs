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
use crate::canvas::Origin;
use crate::color;
use crate::config;
use crate::net;
use crate::protocol::{Author, CanvasOp, Stroke, StrokePoint};
use crate::shapes::{self, Shape};

const INSTRUCTIONS: &str = "\
手書きのキャンバスを読み書きできます。キャンバスはブラウザで開くページで、利用者は iPad・スマートフォン・PC などから書く。
- 座標はキャンバス左上を原点とする CSS ピクセル。x は右、y は下が正。pressure は筆圧(0〜1)で、線の太さになる
- 複数の機器から同時に繋がることがある。筆跡は全部で共有し、紙の大きさと見えている範囲は機器ごとに違う。canvases は繋がっているページの一覧(id・ip・page・viewport)で、空ならどれも繋がっていない
- page はそのページの紙の大きさ。紙は (0, 0) から (width, height) まで。ページの画面の大きさと同じで、利用者は紙の中だけを拡大して見る
- viewport は、ツールを呼んだ時点でそのページに見えている範囲(x, y が左上、width と height が大きさ)。利用者が拡大・移動すると変わる。書き込むときは見せたいページの viewport に収める(すべてのページに見せるなら、すべての viewport が重なる範囲に収める)
- read_canvas と read_canvas_changes は、キャンバスの画像(PNG)と、筆跡ごとの ID・author・範囲(bounds。線の太さを含めて外側に丸めた整数)を返す。点の座標は返さない
- 画像だけを見ればよいときは read_canvas に include_strokes: false を渡すと、筆跡ごとの一覧を省ける
- 画像の左上はキャンバスの (image.x, image.y)。キャンバスの 1 CSS ピクセルが画像の image.scale ピクセル
- 点の座標が要るときは read_strokes に ID を渡す
- 図形(折れ線・長方形・楕円)と文字は draw_shapes で書く。点の列を送らずに済む。文字は KanjiVG の字形(漢字・かな・英数字・記号)で書き、字形の無い文字は missing に返す
- 手書きのような自由な線だけを draw_strokes で書く
- 線の色は color で、#rrggbb の形(例: #ff3b30)。省略すると黒
- 筆跡は strokes の後ろほど上に重なる
- 変更を追うときは read_canvas の revision を覚えておき、read_canvas_changes に渡す。ops が空なら変更は無く、画像も付かない
- AI が消したり直したりできるのは、AI が書いた筆跡(author が ai)だけ
- キャンバスが繋がっていなければ(canvases が空なら) connect_canvas で接続する";

#[derive(Deserialize, JsonSchema)]
struct ReadCanvasParams {
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
}

#[derive(Deserialize, JsonSchema)]
struct DrawShapesParams {
    /// Shapes to write. Later ones are stacked on top
    shapes: Vec<Shape>,
}

#[derive(Serialize)]
struct DrawShapesResult {
    revision: u64,
    /// IDs of the strokes written for each shape (same order as shapes)
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

    /// Stacks the strokes on top as AI strokes and returns their IDs
    fn insert_ai_strokes(&self, strokes: Vec<(String, Vec<StrokePoint>)>) -> Vec<String> {
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
                let index = usize::MAX; // Stack on top
                self.app
                    .canvas
                    .apply(Origin::Ai, CanvasOp::Insert { stroke, index });
                id
            })
            .collect()
    }

    /// Picks out only the strokes written by the AI
    fn split_ai_owned(&self, ids: Vec<String>) -> (Vec<String>, Vec<String>) {
        ids.into_iter().partition(|id| {
            self.app
                .canvas
                .stroke(id)
                .is_some_and(|stroke| stroke.author == Author::Ai)
        })
    }

    #[tool(
        description = "キャンバス全体を読む。画像(PNG)と、revision・繋がっているページごとの紙の大きさと見えている範囲 canvases・画像の位置 image・筆跡ごとの ID と範囲 strokes を返す。include_strokes: false なら strokes を省く"
    )]
    fn read_canvas(
        &self,
        Parameters(params): Parameters<ReadCanvasParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let include_strokes = params.include_strokes.unwrap_or(true);
        view::canvas_result(&self.app.canvas.snapshot(), include_strokes)
    }

    #[tool(
        description = "指定した revision より後の変更を読む。kind が ops なら順に適用すると今の状態になる変更(筆跡は ID と範囲だけ)、snapshot なら古すぎるため全体。繋がっているページごとの紙の大きさと今見えている範囲 canvases はいつも返す。変更があれば今のキャンバスの画像も返す(ops が空なら変更なしで、画像は無い)"
    )]
    fn read_canvas_changes(
        &self,
        Parameters(params): Parameters<ChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let changes = self.app.canvas.changes_since(params.since_revision);
        // Taken after the diff, so the image is the same revision as the diff or newer (see image.revision)
        let current = self.app.canvas.snapshot();
        view::changes_result(&changes, &current)
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
            match self.app.canvas.stroke(&id) {
                Some(stroke) => result.strokes.push(stroke),
                None => result.missing.push(id),
            }
        }
        json_result(&result)
    }

    #[tool(
        description = "キャンバスに線を書く(キャンバスのページにも表示される)。書いた筆跡の ID を返す"
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
        let ids = self.insert_ai_strokes(strokes);
        json_result(&WriteResult {
            revision: self.revision(),
            ids,
            skipped: Vec::new(),
        })
    }

    #[tool(
        description = "折れ線(line)・長方形(rect)・楕円(ellipse)・文字(text)をキャンバスに書く(キャンバスのページにも表示される)。点の列を送らずに済む。図形ごとに書いた筆跡の ID を返す。文字は字形の画ごとに筆跡になる。字形の無い文字は書かずに空け、missing に返す"
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
        let ids = groups
            .into_iter()
            .map(|strokes| self.insert_ai_strokes(strokes))
            .collect();
        json_result(&DrawShapesResult {
            revision: self.revision(),
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
        if let Some(current) = owned.first().and_then(|id| self.app.canvas.stroke(id)) {
            let stroke = Stroke {
                color: checked_color(params.color, &current.color)?,
                points: checked_points(params.points)?,
                ..current
            };
            self.app
                .canvas
                .apply(Origin::Ai, CanvasOp::Replace { stroke });
        }
        json_result(&WriteResult {
            revision: self.revision(),
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
        if !owned.is_empty() {
            let ids = owned.clone();
            self.app.canvas.apply(Origin::Ai, CanvasOp::Remove { ids });
        }
        json_result(&WriteResult {
            revision: self.revision(),
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
