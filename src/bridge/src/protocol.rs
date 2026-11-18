//! Format of the data exchanged between the canvas page and the bridge.
//!
//! The TypeScript types (`src/ui/src/features/sync/protocol.ts`) are generated from these definitions.
//! After changing a definition, regenerate with `UPDATE_PROTOCOL_TS=1 cargo test protocol`
//! (if they drift without regenerating, `typescript_definition_is_up_to_date` fails).

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

use crate::color;

/// Version of the page–bridge protocol. The page sends it in hello, and the bridge rejects a different one.
/// Pages from before sheets do not send it, so it is taken as 0.
/// From 2, operations carry a seq and the bridge returns an ack
pub const PROTOCOL_VERSION: u32 = 2;
/// Maximum number of sheets
pub const MAX_SHEETS: usize = 50;
/// ID of the first sheet, and of the sheet left after deleting all sheets. The bridge and the page start with
/// the same ID, so after the bridge restarts it matches the first sheet of a reconnecting page
pub const INITIAL_SHEET_ID: &str = "sheet-1";

fn default_color() -> String {
    color::BLACK.into()
}

// The /// doc comments are sent to the AI as tool parameter descriptions, so they are written in English
/// One point of a stroke. Coordinates are CSS pixels with the origin at the top left of the canvas
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    /// Pen pressure (0 to 1)
    pub pressure: f64,
}

/// Whether a stroke was written by a human or the AI
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    Human,
    Ai,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stroke {
    /// ID unique across the whole canvas
    pub id: String,
    pub author: Author,
    /// Stroke color ("#rrggbb"). Absent on strokes from before colors existed, treated as black
    #[serde(default = "default_color")]
    pub color: String,
    pub points: Vec<StrokePoint>,
}

/// Position and size of an image (canvas coordinates, CSS pixels). Images are placed under all strokes.
/// Only one per canvas for now (referenced by ID, anticipating several in the future)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImagePlacement {
    /// ID unique across the canvas. Shared with the bytes, which are sent on a separate stream (ImageStream)
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Contents of one sheet. The paper size (PageSize) is each connection's screen size and is not kept per sheet
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sheet {
    /// ID unique across the whole canvas
    pub id: String,
    /// Stacking order (later strokes are on top)
    pub strokes: Vec<Stroke>,
    /// Image placed under the strokes (one per sheet). None if there is none
    #[serde(default)]
    pub image: Option<ImagePlacement>,
}

/// A change to the canvas. Used both page → bridge and bridge → page.
/// Stroke and image changes carry the sheet they apply to in `sheet` (the sheet ID)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CanvasOp {
    /// Inserts a stroke at position index (stacking order) on that sheet. Out of range puts it at the end
    Insert {
        sheet: String,
        stroke: Stroke,
        index: usize,
    },
    /// Removes strokes from that sheet. Missing IDs are ignored
    Remove { sheet: String, ids: Vec<String> },
    /// Replaces the stroke with the same ID on that sheet, keeping the stacking order
    Replace { sheet: String, stroke: Stroke },
    /// Places the image on that sheet (also used to move or resize it). Replaces any existing image.
    /// The bytes are sent on a separate stream (ImageStream)
    SetImage {
        sheet: String,
        image: ImagePlacement,
    },
    /// Removes the image from that sheet. Ignored if there is no image
    RemoveImage { sheet: String },
    /// Inserts an empty sheet at position index. Out of range puts it at the end
    AddSheet { id: String, index: usize },
    /// Removes a sheet with its contents
    RemoveSheet { id: String },
    /// Changes the current sheet
    SelectSheet { id: String },
    /// Removes all sheets and makes one empty `INITIAL_SHEET_ID` sheet the current sheet
    ResetSheets,
}

/// Header on the first line of a separate stream that carries image bytes (opened one-to-one, apart from the bidirectional stream).
/// provide is followed by the PNG bytes; request has nothing after it. Either the page or the bridge can open one
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ImageStream {
    /// Sends the bytes of the image with the given ID
    Provide { id: String },
    /// The bytes of the image with the given ID are requested. Not sent if it does not have them
    Request { id: String },
}

/// Area currently visible on the canvas page (canvas coordinates, CSS pixels). Changes with zoom and pan
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Viewport {
    /// Top left of the visible area. Canvas pages from before zoom support do not send it, so it is treated as 0
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Size of the canvas (paper) in CSS pixels. The paper starts at (0, 0) and matches the canvas area on the page (what is visible at 100%)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

/// Canvas page → bridge. One JSON object per line
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    /// Sent first after connecting. token is the one-time token from the QR code URL
    Hello {
        token: String,
        viewport: Viewport,
        /// Paper size. Canvas pages from before paper sizes existed do not send it
        #[serde(default)]
        page: Option<PageSize>,
        /// Resume key (resumeKey) received on a previous connection. Not sent if there is none
        #[serde(default)]
        resume: Option<String>,
        /// Protocol version. Pages from before sheets do not send it, so it is taken as 0
        #[serde(default)]
        protocol: u32,
    },
    /// The visible area or paper size changed (screen size, zoom, pan)
    Resize {
        viewport: Viewport,
        #[serde(default)]
        page: Option<PageSize>,
    },
    /// An operation made on the page. seq is the number the page gives each operation (increasing within the page).
    /// After handling the operation (applied or ignored), the bridge returns an `Ack` with the same number.
    /// After reconnecting, the page resends the operations without an ack, with the same numbers
    Op { op: CanvasOp, seq: u64 },
    /// Send to AI was pressed. Tells the AI (MCP's wait_for_canvas)
    SendToAi,
}

/// Bridge → canvas page. One JSON object per line
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    /// The token is valid; waiting for approval on the PC
    AwaitingApproval,
    /// The connection was refused. The connection is closed after this
    Rejected {
        reason: String,
    },
    /// Resume key. Sent when approved (followed by snapshot) and when the key is replaced while connected.
    /// Sending the last received key with hello reconnects without waiting for approval on the PC (usable once)
    ResumeKey {
        key: String,
    },
    /// Another connection reconnected with the key handed to this connection. The connection is closed after this
    Superseded,
    /// The connection was closed from the PC's menu. The connection is closed after this
    Disconnected,
    /// Approved. The whole canvas the bridge has (the list of sheets and the current sheet ID). The page replaces its own with it,
    /// then reapplies and resends its operations that have no ack yet.
    /// If the page lacks the bytes of a bridge image, it receives them with an ImageStream request
    Snapshot {
        /// Sheets (in stacking order)
        sheets: Vec<Sheet>,
        /// ID of the current sheet
        current: String,
    },
    Op {
        op: CanvasOp,
    },
    /// Reports that an `Op` was handled (applied or ignored). The page removes the operations up to
    /// this number from those to resend
    Ack {
        seq: u64,
    },
    /// Whether the AI is waiting in wait_for_canvas (waiting) and whether a Send to AI press has not been
    /// picked up by the AI yet (queued). Sent when syncing starts (right after snapshot) and when it changes
    AiStatus {
        /// The AI is waiting in wait_for_canvas (also true during the grace period after a timeout)
        waiting: bool,
        /// Pressed, but no wait_for_canvas has picked it up yet
        queued: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typescript;

    const TS_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ui/src/features/sync/protocol.ts"
    );

    fn generated() -> String {
        let mut output = typescript::generate(&[
            schemars::schema_for!(ClientMessage),
            schemars::schema_for!(ServerMessage),
            // Not referenced from the message types, but the page uses it for the stream header
            schemars::schema_for!(ImageStream),
        ]);
        // Append the constants after the types (doc is the same text as the Rust comment)
        output.push_str(&typescript::constant(
            "PROTOCOL_VERSION",
            "Version of the page–bridge protocol. The page sends it in hello, and the bridge rejects a different one.\nPages from before sheets do not send it, so it is taken as 0.\nFrom 2, operations carry a seq and the bridge returns an ack",
            &serde_json::to_value(PROTOCOL_VERSION).unwrap(),
        ));
        output.push_str(&typescript::constant(
            "MAX_SHEETS",
            "Maximum number of sheets",
            &serde_json::to_value(MAX_SHEETS).unwrap(),
        ));
        output.push_str(&typescript::constant(
            "INITIAL_SHEET_ID",
            "ID of the first sheet, and of the sheet left after deleting all sheets. The bridge and the page start with\nthe same ID, so after the bridge restarts it matches the first sheet of a reconnecting page",
            &serde_json::to_value(INITIAL_SHEET_ID).unwrap(),
        ));
        output
    }

    #[test]
    fn typescript_definition_is_up_to_date() {
        let expected = generated();
        if std::env::var_os("UPDATE_PROTOCOL_TS").is_some() {
            std::fs::write(TS_PATH, &expected).unwrap();
        }
        let actual = std::fs::read_to_string(TS_PATH).unwrap_or_default();
        assert!(
            actual == expected,
            "{TS_PATH} が古い。UPDATE_PROTOCOL_TS=1 cargo test protocol で書き直す"
        );
    }

    #[test]
    fn old_canvas_page_messages_still_parse() {
        // Canvas pages from before zoom support do not send viewport x, y or page
        let message: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "resize",
            "viewport": { "width": 800.0, "height": 600.0 }
        }))
        .unwrap();
        let ClientMessage::Resize { viewport, page } = message else {
            panic!("resize のはず");
        };
        assert_eq!((viewport.x, viewport.y), (0.0, 0.0));
        assert_eq!(page, None);
    }

    #[test]
    fn sheet_ops_use_camel_case() {
        assert_eq!(
            serde_json::to_value(CanvasOp::AddSheet {
                id: "s".into(),
                index: 1
            })
            .unwrap(),
            serde_json::json!({ "type": "addSheet", "id": "s", "index": 1 })
        );
        assert_eq!(
            serde_json::to_value(CanvasOp::ResetSheets).unwrap(),
            serde_json::json!({ "type": "resetSheets" })
        );
        assert_eq!(
            serde_json::to_value(CanvasOp::Insert {
                sheet: "s".into(),
                stroke: Stroke {
                    id: "st".into(),
                    author: Author::Human,
                    color: color::BLACK.into(),
                    points: vec![]
                },
                index: 0
            })
            .unwrap(),
            serde_json::json!({
                "type": "insert",
                "sheet": "s",
                "stroke": {
                    "id": "st",
                    "author": "human",
                    "color": "#000000",
                    "points": []
                },
                "index": 0
            })
        );
        assert_eq!(
            serde_json::to_value(CanvasOp::SetImage {
                sheet: "s".into(),
                image: ImagePlacement {
                    id: "img".into(),
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0
                }
            })
            .unwrap(),
            serde_json::json!({
                "type": "setImage",
                "sheet": "s",
                "image": { "id": "img", "x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0 }
            })
        );
        assert_eq!(
            serde_json::to_value(CanvasOp::RemoveImage { sheet: "s".into() }).unwrap(),
            serde_json::json!({ "type": "removeImage", "sheet": "s" })
        );

        assert_eq!(
            serde_json::to_value(ImageStream::Request { id: "img".into() }).unwrap(),
            serde_json::json!({ "type": "request", "id": "img" })
        );
    }

    #[test]
    fn hello_without_protocol_is_zero() {
        let message: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello",
            "token": "t",
            "viewport": { "width": 800.0, "height": 600.0 }
        }))
        .unwrap();
        let ClientMessage::Hello { protocol, .. } = message else {
            panic!("hello のはず");
        };
        assert_eq!(protocol, 0);
    }

    /// Same shape as `CanvasOp` of old bridges (up to v0.3.0, before sheets).
    /// Only checks that it can be read, so the fields' contents are not read
    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "camelCase")]
    enum OldCanvasOp {
        Insert { stroke: Stroke, index: usize },
        Remove { ids: Vec<String> },
        Replace { stroke: Stroke },
        SetImage { image: ImagePlacement },
        RemoveImage,
    }

    /// Old bridges skip the unknown field (`sheet`) and can read new pages' ops
    #[test]
    fn old_bridge_reads_new_ops() {
        let insert: OldCanvasOp = serde_json::from_value(
            serde_json::to_value(CanvasOp::Insert {
                sheet: INITIAL_SHEET_ID.into(),
                stroke: Stroke {
                    id: "st".into(),
                    author: Author::Human,
                    color: color::BLACK.into(),
                    points: vec![],
                },
                index: 0,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(insert, OldCanvasOp::Insert { index: 0, .. }));

        let remove_image: OldCanvasOp = serde_json::from_value(
            serde_json::to_value(CanvasOp::RemoveImage {
                sheet: INITIAL_SHEET_ID.into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(remove_image, OldCanvasOp::RemoveImage));
    }

    #[test]
    fn messages_use_camel_case_type_tags() {
        let message = ServerMessage::Op {
            op: CanvasOp::Remove {
                sheet: "s".into(),
                ids: vec!["a".into()],
            },
        };
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({
                "type": "op",
                "op": { "type": "remove", "sheet": "s", "ids": ["a"] }
            })
        );
        assert_eq!(
            serde_json::to_value(ServerMessage::AwaitingApproval).unwrap(),
            serde_json::json!({ "type": "awaitingApproval" })
        );
    }
}
