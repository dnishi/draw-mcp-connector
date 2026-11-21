//! The window that selects the export area (macOS / Windows). Draws the sheet (white background,
//! image, strokes), darkens everything outside the area, and shows the blue border and the handles
//! at the four corners, and the bar at the bottom (the size of the area and the buttons). All the
//! calculations are in area_select.rs; this is only a thin layer that passes tao's events and
//! writes the drawn pixels into softbuffer. Drawing the sheet can be heavy, so it is done on
//! another thread, and the result is returned to the event loop in tray.rs to redraw.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use softbuffer::{Context, Surface};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{ElementState, MouseButton, WindowEvent};
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::keyboard::KeyCode;
use tao::window::{CursorIcon, Window, WindowBuilder, WindowId};
use tracing::warn;

use crate::app::App;
use crate::area_select::{self, Cursor, Outcome, Scene, SheetRenderJob};
use crate::config;
use crate::export_flow::AreaRequest;
use crate::render::{self, Rect};
use crate::tray::UserEvent;

/// The window that selects the export area
pub struct ExportWindow {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    scene: Scene,
    /// The request of this export (the caller passes it to finish)
    request: AreaRequest,
    /// The ratio of physical pixels to logical pixels
    scale_factor: f64,
    /// Sends the result of the sheet drawing on another thread to the event loop
    proxy: EventLoopProxy<UserEvent>,
}

impl ExportWindow {
    /// Opens the window. Draws the current sheet at the moment of opening (the area is the whole sheet at first)
    pub fn open<T: 'static>(
        target: &EventLoopWindowTarget<T>,
        app: &App,
        request: AreaRequest,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self, String> {
        let window = WindowBuilder::new()
            .with_title(format!("{} - Export", config::APP_NAME))
            .with_inner_size(LogicalSize::new(
                area_select::INITIAL_WIDTH,
                area_select::INITIAL_HEIGHT,
            ))
            .with_min_inner_size(LogicalSize::new(
                area_select::MIN_WIDTH,
                area_select::MIN_HEIGHT,
            ))
            .build(target)
            .map_err(|error| error.to_string())?;
        let scale_factor = window.scale_factor();
        let window = Rc::new(window);
        let context = Context::new(window.clone()).map_err(|error| error.to_string())?;
        let surface = Surface::new(&context, window.clone()).map_err(|error| error.to_string())?;

        // Copies the current sheet at the moment of opening (changes after opening are not followed)
        let snapshot = app.canvas.snapshot();
        let sheet = snapshot.current_sheet();
        let image = sheet.image.as_ref().and_then(|placement| {
            app.canvas
                .image_data(&placement.id)
                .map(|data| (placement.clone(), data))
        });
        // The strokes are a copy from when the window opened, shared with Arc (so they are not copied on every draw)
        let scene = Scene::new(
            request.page,
            Arc::new(sheet.strokes.clone()),
            image,
            area_select::INITIAL_WIDTH,
            area_select::INITIAL_HEIGHT,
        );
        let export_window = Self {
            window,
            surface,
            scene,
            request,
            scale_factor,
            proxy,
        };
        export_window.bring_to_front();
        Ok(export_window)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// The request of this export
    pub fn request(&self) -> AreaRequest {
        self.request
    }

    /// The current area (CSS pixels)
    pub fn area(&self) -> Rect {
        self.scene.area()
    }

    /// Brings it to the front so it can receive keys (the same way as qr_window.rs)
    pub fn bring_to_front(&self) {
        self.window.set_focus();
        self.window.request_redraw();
    }

    /// Passes the events of the window to area_select. Once it becomes Export or Cancel,
    /// returns it (closing the window and calling `export_flow::finish` is the caller's job)
    pub fn event(&mut self, event: &WindowEvent) -> Option<Outcome> {
        let outcome = match event {
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = to_logical(position, self.scale_factor);
                self.scene.move_mouse(x, y)
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => self.scene.press(),
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Released,
                ..
            } => self.scene.release(),
            // Only when pressed (not on repeat)
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let key = match event.physical_key {
                    KeyCode::Enter => area_select::Key::Enter,
                    KeyCode::Escape => area_select::Key::Escape,
                    _ => return None,
                };
                self.scene.key(key)
            }
            WindowEvent::Resized(size) => self.scene.resize(
                size.width as f64 / self.scale_factor,
                size.height as f64 / self.scale_factor,
            ),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = *scale_factor;
                let size = self.window.inner_size();
                self.scene.resize(
                    size.width as f64 / *scale_factor,
                    size.height as f64 / *scale_factor,
                )
            }
            // Closing the window (×) is Cancel
            WindowEvent::CloseRequested => Outcome::Cancel,
            _ => Outcome::None,
        };
        if outcome != Outcome::None {
            self.set_cursor();
            if matches!(outcome, Outcome::Redraw) {
                self.window.request_redraw();
            }
        }
        matches!(outcome, Outcome::Export | Outcome::Cancel).then_some(outcome)
    }

    /// Sets the cursor to the shape area_select decided
    fn set_cursor(&self) {
        let icon = match self.scene.cursor() {
            Cursor::NwseResize => CursorIcon::NwseResize,
            Cursor::NeswResize => CursorIcon::NeswResize,
            Cursor::Move => CursorIcon::Move,
            Cursor::Crosshair => CursorIcon::Crosshair,
            Cursor::Hand => CursorIcon::Hand,
            Cursor::Default => CursorIcon::Default,
        };
        self.window.set_cursor_icon(icon);
    }

    /// Redraws when the result of the sheet drawing on another thread arrives and it matches (called through tray.rs)
    pub fn sheet_rendered(&mut self, job: SheetRenderJob, pixels: render::Pixels) {
        if self
            .scene
            .finish_sheet_render(&job, self.scale_factor, pixels)
        {
            self.window.request_redraw();
        }
    }

    /// Redraws (on RedrawRequested)
    pub fn redraw(&mut self) {
        // When the layout changed (opening, or the size or scale changed),
        // starts the sheet drawing on another thread
        self.start_sheet_render();
        let size = self.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return; // The size is 0, for example when minimized
        };
        if let Err(error) = self.surface.resize(width, height) {
            warn!("範囲を選ぶウィンドウの大きさを決められない: {error}");
            return;
        }
        let mut buffer = match self.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(error) => {
                warn!("範囲を選ぶウィンドウを描けない: {error}");
                return;
            }
        };
        // The pixels are u32 as `0x00RRGGBB` (the form softbuffer uses)
        let pixels = self
            .scene
            .paint(width.get(), height.get(), self.scale_factor);
        let target: &mut [u32] = &mut buffer;
        target.copy_from_slice(&pixels);
        if let Err(error) = buffer.present() {
            warn!("範囲を選ぶウィンドウを描けない: {error}");
        }
    }

    /// Starts drawing the sheet of the current layout on another thread if it is not drawn yet.
    /// Does not start a drawing with the same key twice (if the layout changes while drawing,
    /// it starts a drawing with the new key. The old drawing does not need to be stopped; when
    /// it finishes, its key does not match, so it is discarded)
    fn start_sheet_render(&mut self) {
        let Some(job) = self.scene.sheet_render_job(self.scale_factor) else {
            return;
        };
        // The strokes and the image are the copies from when the window opened, shared with Arc, not duplicated
        let strokes = self.scene.sheet_strokes().clone();
        let image = self.scene.sheet_image();
        let proxy = self.proxy.clone();
        let window_id = self.window.id();
        std::thread::spawn(move || {
            let image = image
                .as_ref()
                .map(|(placement, data)| (placement, data.as_ref()));
            let pixels =
                render::render_area(strokes.as_slice(), image, Rect::from(job.page), job.scale);
            let _ = proxy.send_event(UserEvent::SheetRendered(window_id, job, pixels));
        });
    }
}

/// Converts physical pixels to logical pixels
fn to_logical(position: &PhysicalPosition<f64>, scale_factor: f64) -> (f64, f64) {
    (position.x / scale_factor, position.y / scale_factor)
}
