//! State shared across the whole bridge.

use std::sync::Arc;

use crate::ai_inbox::AiInbox;
use crate::canvas::Canvas;
use crate::config;
use crate::disconnect::DisconnectRequests;
use crate::link_status::LinkStatusCell;
use crate::pairing::Pairing;
use crate::resume::ResumeKeys;
use crate::store::Store;
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
    /// Saves the canvas state (None when there is no place to save it)
    pub store: Option<Arc<Store>>,
}

impl App {
    pub fn new() -> Self {
        Self::build(Canvas::new(), None)
    }

    /// An App that saves the canvas state in the data folder. Starts from the saved state if there is one
    pub fn with_store(store: Store) -> Self {
        let store = Arc::new(store);
        let canvas = match store.load() {
            Some(saved) => Canvas::restored(saved),
            None => Canvas::new(),
        };
        // Without recording the loaded images as written, the first save after launch would rewrite them all
        store.remember_images(&canvas);
        Self::build(canvas, Some(store))
    }

    /// Writes the current state to the file right away (used by Quit). If the save task is writing,
    /// waits on the Store's lock for it to finish before writing
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn save_now(&self) {
        if let Some(store) = &self.store {
            store.save(&self.canvas);
        }
    }

    fn build(canvas: Canvas, store: Option<Arc<Store>>) -> Self {
        Self {
            canvas,
            pairing: Pairing::new(),
            resume_keys: ResumeKeys::new(config::RESUME_KEY_LIFETIME),
            status: LinkStatusCell::default(),
            disconnects: DisconnectRequests::default(),
            ai_inbox: AiInbox::new(config::AI_WAITING_GRACE),
            ui: Ui::default(),
            store,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::INITIAL_SHEET_ID;
    use crate::store::tests::{canvas_with_image, cleanup, temp_dir};

    #[test]
    fn with_store_starts_from_the_saved_state() {
        let dir = temp_dir();
        let canvas = canvas_with_image("img", &[9, 9]);
        let saved = canvas.snapshot();
        Store::new(dir.clone()).save(&canvas);

        let app = App::with_store(Store::new(dir.clone()));
        let restored = app.canvas.snapshot();
        assert_eq!(restored.revision, saved.revision);
        assert_eq!(restored.current, saved.current);
        assert_eq!(restored.sheets, saved.sheets);
        assert!(app.store.is_some());

        cleanup(&dir);
    }

    #[test]
    fn with_store_starts_blank_when_nothing_is_saved() {
        let dir = temp_dir();
        let app = App::with_store(Store::new(dir.clone()));
        let snapshot = app.canvas.snapshot();
        assert_eq!(snapshot.current, INITIAL_SHEET_ID);
        assert_eq!(snapshot.sheets.len(), 1);
        assert!(snapshot.sheets[0].strokes.is_empty());
        assert!(app.store.is_some());

        cleanup(&dir);
    }

    #[test]
    fn save_now_writes_the_current_state() {
        let dir = temp_dir();
        let app = App::with_store(Store::new(dir.clone()));
        app.save_now();
        assert!(dir.join("canvas.json").exists());

        cleanup(&dir);
    }
}
