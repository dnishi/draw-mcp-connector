//! State shared across the whole bridge.

use crate::canvas::Canvas;
use crate::config;
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
    pub ui: Ui,
}

impl App {
    pub fn new() -> Self {
        Self {
            canvas: Canvas::new(),
            pairing: Pairing::new(),
            resume_keys: ResumeKeys::new(config::RESUME_KEY_LIFETIME),
            status: LinkStatusCell::default(),
            ui: Ui::default(),
        }
    }
}
