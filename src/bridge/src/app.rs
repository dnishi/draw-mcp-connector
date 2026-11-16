//! State shared across the whole bridge.

use crate::ai_inbox::AiInbox;
use crate::canvas::Canvas;
use crate::config;
use crate::disconnect::DisconnectRequests;
use crate::link_status::LinkStatusCell;
use crate::pairing::Pairing;
use crate::resume::ResumeKeys;
use crate::ui::Ui;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct App {
    pub canvas: Canvas,
    pub pairing: Pairing,
    pub resume_keys: ResumeKeys,
    pub status: LinkStatusCell,
    /// Where connection tasks are asked to close (the menu's Disconnect)
    pub disconnects: DisconnectRequests,
    /// Hands presses of the canvas's Send to AI to MCP's wait_for_canvas
    pub ai_inbox: AiInbox,
    pub ui: Ui,
}

impl App {
    pub fn new() -> Self {
        Self {
            canvas: Canvas::new(),
            pairing: Pairing::new(),
            resume_keys: ResumeKeys::new(config::RESUME_KEY_LIFETIME),
            status: LinkStatusCell::default(),
            disconnects: DisconnectRequests::default(),
            ai_inbox: AiInbox::new(config::AI_WAITING_GRACE),
            ui: Ui::default(),
        }
    }
}
