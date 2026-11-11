//! QR code window (macOS / Windows). It contains only the QR code, drawn pixel by pixel with softbuffer.
//! Created, owned, and redrawn by the main-thread event loop (tray.rs).

use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, Surface};
use tao::dpi::LogicalSize;
use tao::event_loop::EventLoopWindowTarget;
use tao::window::{Window, WindowBuilder, WindowId};
use tracing::warn;

use crate::app::App;
use crate::config;
use crate::qr::QrCode;

/// Initial window size (logical pixels). Even at QR version 10 (97 modules plus the quiet zone),
/// each module is at least 3 logical pixels
const INITIAL_SIZE: f64 = 360.0;

pub struct QrWindow {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
}

impl QrWindow {
    pub fn open<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Self, String> {
        let window = WindowBuilder::new()
            .with_title(format!("{} - Scan to Open the Canvas", config::APP_NAME))
            .with_inner_size(LogicalSize::new(INITIAL_SIZE, INITIAL_SIZE))
            .with_minimizable(false)
            .with_maximizable(false)
            .build(target)
            .map_err(|error| error.to_string())?;
        let window = Rc::new(window);
        let context = Context::new(window.clone()).map_err(|error| error.to_string())?;
        let surface = Surface::new(&context, window.clone()).map_err(|error| error.to_string())?;
        let qr_window = Self { window, surface };
        qr_window.bring_to_front();
        Ok(qr_window)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Brings the window to the front and redraws it (the QR code can change when the certificate is regenerated)
    pub fn bring_to_front(&self) {
        self.window.set_focus();
        self.window.request_redraw();
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn redraw(&mut self, app: &App) {
        if let Err(error) = self.draw(app) {
            warn!("QR コードを描けない: {error}");
        }
    }

    fn draw(&mut self, app: &App) -> Result<(), String> {
        let size = self.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(()); // Size is 0, e.g. when minimized
        };
        self.surface
            .resize(width, height)
            .map_err(|error| error.to_string())?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;

        // Show it blank while there is no endpoint yet (right after launch) or the URL is too long
        let qr = app
            .pairing
            .canvas_url()
            .and_then(|url| QrCode::encode(url.as_str()).ok());
        match qr {
            Some(qr) => qr.render(&mut buffer, width.get() as usize, height.get() as usize),
            None => buffer.fill(0x00ff_ffff),
        }
        buffer.present().map_err(|error| error.to_string())
    }
}
