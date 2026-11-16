//! Format of the data exchanged between the canvas page and the bridge.
//!
//! The TypeScript types (`src/ui/src/features/sync/protocol.ts`) are generated from these definitions.
//! After changing a definition, regenerate with `UPDATE_PROTOCOL_TS=1 cargo test protocol`
//! (if they drift without regenerating, `typescript_definition_is_up_to_date` fails).

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

use crate::color;

fn default_color() -> String {
    color::BLACK.into()
}

/// A point of a stroke. Coordinates are CSS pixels with the origin at the top left of the canvas
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

/// A change to the canvas. Used both page → bridge and bridge → page
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CanvasOp {
    /// Inserts a stroke at position index (stacking order). Appends it if out of range
    Insert { stroke: Stroke, index: usize },
    /// Removes strokes. Unknown IDs are ignored
    Remove { ids: Vec<String> },
    /// Replaces the stroke with the same ID, keeping its stacking order
    Replace { stroke: Stroke },
    /// Places an image (also used to replace it with a new position or size). Replaces any existing image.
    /// The bytes are sent on a separate stream (ImageStream)
    SetImage { image: ImagePlacement },
    /// Removes the image. Ignored if there is no image
    RemoveImage,
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
    },
    /// The visible area or paper size changed (screen size, zoom, pan)
    Resize {
        viewport: Viewport,
        #[serde(default)]
        page: Option<PageSize>,
    },
    Op {
        op: CanvasOp,
    },
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
    /// Approved. The whole canvas the bridge has. The page matches it,
    /// keeps strokes only it has (such as ones written before the bridge restarted), and sends them back with insert.
    /// The bridge's image wins, but an image the page has and the bridge does not is sent back with setImage.
    /// If the page does not have the bytes of the bridge's image, it receives them with an ImageStream request
    Snapshot {
        strokes: Vec<Stroke>,
        /// The placed image. None if there is none
        #[serde(default)]
        image: Option<ImagePlacement>,
    },
    Op {
        op: CanvasOp,
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
        typescript::generate(&[
            schemars::schema_for!(ClientMessage),
            schemars::schema_for!(ServerMessage),
            // Not referenced from the message types, but the page uses it for the stream header
            schemars::schema_for!(ImageStream),
        ])
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
    fn image_ops_use_camel_case_and_snapshot_image_is_optional() {
        let message = ServerMessage::Op {
            op: CanvasOp::SetImage {
                image: ImagePlacement {
                    id: "img".into(),
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({
                "type": "op",
                "op": {
                    "type": "setImage",
                    "image": { "id": "img", "x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0 }
                }
            })
        );
        assert_eq!(
            serde_json::to_value(CanvasOp::RemoveImage).unwrap(),
            serde_json::json!({ "type": "removeImage" })
        );

        // Snapshots from pages before images were introduced still parse
        let snapshot: ServerMessage = serde_json::from_value(serde_json::json!({
            "type": "snapshot",
            "strokes": []
        }))
        .unwrap();
        let ServerMessage::Snapshot { image, .. } = snapshot else {
            panic!("snapshot のはず");
        };
        assert_eq!(image, None);

        assert_eq!(
            serde_json::to_value(ImageStream::Request { id: "img".into() }).unwrap(),
            serde_json::json!({ "type": "request", "id": "img" })
        );
    }

    #[test]
    fn messages_use_camel_case_type_tags() {
        let message = ServerMessage::Op {
            op: CanvasOp::Remove {
                ids: vec!["a".into()],
            },
        };
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({ "type": "op", "op": { "type": "remove", "ids": ["a"] } })
        );
        assert_eq!(
            serde_json::to_value(ServerMessage::AwaitingApproval).unwrap(),
            serde_json::json!({ "type": "awaitingApproval" })
        );
    }
}
