//! The calculations for the window that selects the export area.
//!
//! The window itself (tao) and the drawing to the screen (softbuffer) live in
//! export_window.rs and are not compiled on development Linux. So the layout,
//! hit testing, area manipulation, and pixel drawing calculations are all done
//! here, so they can be tested on Linux as well.
//!
//! Screen coordinates are logical pixels with the top left corner of the window
//! as the origin, the area is in sheet CSS pixels (`render::Rect`), and the
//! pixels that are drawn are physical pixels (u32 as `0x00RRGGBB`, the form
//! softbuffer uses)
// export_window.rs is not compiled on development Linux, so the parts used only
// by the window side are treated as dead code (they are used on macOS / Windows)
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::sync::Arc;

use crate::protocol::{Author, ImagePlacement, PageSize, Stroke, StrokePoint};
use crate::render::{self, Rect, StoredImage};
use crate::shapes::{self, Shape, ShapeKind};

/// The initial size of the window (logical pixels)
pub const INITIAL_WIDTH: f64 = 960.0;
pub const INITIAL_HEIGHT: f64 = 720.0;
/// The minimum size of the window (logical pixels)
pub const MIN_WIDTH: f64 = 480.0;
pub const MIN_HEIGHT: f64 = 360.0;
/// The height of the bar at the bottom (logical pixels)
pub const BAND_HEIGHT: f64 = 56.0;
/// The margin around the sheet (logical pixels)
pub const PADDING: f64 = 16.0;
/// One side of a corner handle (logical pixels)
pub const HANDLE: f64 = 10.0;
/// The width of the area border (physical pixels)
pub const BORDER: u32 = 2;
/// The text size of the bar (logical pixels)
pub const FONT_SIZE: f64 = 16.0;
/// The height of the buttons (logical pixels)
pub const BUTTON_HEIGHT: f64 = 32.0;
/// The left and right padding of the button text (logical pixels)
pub const BUTTON_PADDING: f64 = 10.0;
/// The gap between the buttons and on the right of the bar (logical pixels)
pub const BUTTON_GAP: f64 = 8.0;

/// The background (light gray)
pub const BACKGROUND: u32 = 0x00e5e5e5;
/// The line on the top edge of the bar and the button borders (gray)
pub const LINE_GRAY: u32 = 0x00d4d4d4;
/// The area border and the edges of the handles, the Export button (blue)
pub const BLUE: u32 = 0x000ea5e9;
/// The fill of a button under the mouse (light gray)
pub const HOVER: u32 = 0x00f5f5f5;
/// White (the fill of the bar, buttons, and handles)
pub const WHITE: u32 = 0x00ffffff;

/// The pressure used when drawing as a line (ShapeKind::Rect) (line width 1.5 logical pixels)
const LINE_PRESSURE: f64 = 0.25;
/// The color of the text
const TEXT: u32 = 0x00262626;

/// Turn the color of a screen pixel (0x00RRGGBB) into the form of a stroke color ("#rrggbb")
fn stroke_color(color: u32) -> String {
    format!("#{color:06x}")
}

/// A rectangle (logical pixels, with the top left corner of the window as the origin)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Region {
    const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// Whether a point is contained (the edges are included)
    fn contains(&self, x: f64, y: f64) -> bool {
        self.x <= x && x <= self.x + self.w && self.y <= y && y <= self.y + self.h
    }
}

/// The buttons of the bar at the bottom (from the left)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Return the area to the whole sheet
    SelectAll,
    /// Discard the export
    Cancel,
    /// Finish the export with the area
    Export,
}

impl Button {
    /// From left to right
    pub const ALL: [Button; 3] = [Button::SelectAll, Button::Cancel, Button::Export];

    pub fn label(self) -> &'static str {
        match self {
            Button::SelectAll => "Select All",
            Button::Cancel => "Cancel",
            Button::Export => "Export",
        }
    }
}

/// The layout of the screen (logical pixels)
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// The position and size of the sheet on the screen (the width and height are 0 when the sheet cannot be drawn)
    pub sheet_area: Region,
    /// The logical pixels per 1 CSS pixel of the sheet (the scale when drawing)
    pub scale: f64,
    /// The positions of the buttons (in the order of `Button::ALL`)
    pub buttons: [Region; 3],
    /// The top left corner of the area size text on the left of the bar
    pub size_text: (f64, f64),
}

/// Compute the layout from the size of the sheet and the size of the window (logical pixels)
pub fn layout(page: PageSize, width: f64, height: f64) -> Layout {
    // Where the sheet fits (above the bar, excluding the margin)
    let usable_w = width - PADDING * 2.0;
    let usable_h = height - BAND_HEIGHT - PADDING * 2.0;
    // The largest scale that fits while keeping the aspect ratio
    let scale = if usable_w > 0.0 && usable_h > 0.0 && page.width > 0.0 && page.height > 0.0 {
        (usable_w / page.width).min(usable_h / page.height)
    } else {
        0.0
    };
    let sheet_area = if scale > 0.0 {
        let (w, h) = (page.width * scale, page.height * scale);
        Region::new((width - w) / 2.0, (height - BAND_HEIGHT - h) / 2.0, w, h)
    } else {
        Region::ZERO
    };

    // The 3 buttons on the right of the bar (right-aligned. Placed in order from the right)
    let band_top = height - BAND_HEIGHT;
    let mut x = width - PADDING;
    let mut buttons = [Region::ZERO; 3];
    for index in (0..Button::ALL.len()).rev() {
        let button = Button::ALL[index];
        let w = text_ink_width(button.label(), FONT_SIZE) + BUTTON_PADDING * 2.0;
        x -= w;
        buttons[index] = Region::new(
            x,
            band_top + (BAND_HEIGHT - BUTTON_HEIGHT) / 2.0,
            w,
            BUTTON_HEIGHT,
        );
        x -= BUTTON_GAP;
    }
    // The text on the left of the bar (a 16px high box centered in the bar)
    let size_text = (PADDING, band_top + (BAND_HEIGHT - FONT_SIZE) / 2.0);
    Layout {
        sheet_area,
        scale,
        buttons,
        size_text,
    }
}

/// The ink width of the text (logical pixels). Decides the width of the buttons
fn text_ink_width(text: &str, size: f64) -> f64 {
    let points: Vec<f64> = text_strokes(text, 0.0, 0.0, size)
        .iter()
        .flatten()
        .map(|point| point.x)
        .collect();
    if points.is_empty() {
        return 0.0;
    }
    points.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - points.iter().cloned().fold(f64::INFINITY, f64::min)
}

/// Turn text into lists of stroke points (coordinates are logical pixels). Since
/// it is the display of the window, it must be drawable
fn text_strokes(text: &str, x: f64, y: f64, size: f64) -> Vec<Vec<StrokePoint>> {
    shapes::outline(&Shape {
        kind: ShapeKind::Text {
            text: text.into(),
            x,
            y,
            size,
        },
        color: None,
        pressure: None,
    })
    .expect("ウィンドウの文字は必ず描ける")
    .strokes
}

/// Turn a shape into strokes (the border of a button)
fn shape_strokes(kind: ShapeKind, color: u32) -> Vec<Stroke> {
    shapes::outline(&Shape {
        kind,
        color: None,
        pressure: Some(LINE_PRESSURE),
    })
    .expect("ボタンの枠は必ず描ける")
    .strokes
    .into_iter()
    .map(|points| window_stroke(points, color))
    .collect()
}

/// A stroke to be drawn inside the window (not a stroke of the canvas, so the ID and the author are only values for drawing)
fn window_stroke(points: Vec<StrokePoint>, color: u32) -> Stroke {
    Stroke {
        id: "export-window".into(),
        author: Author::Human,
        color: stroke_color(color),
        points,
    }
}

/// The shape of the mouse (the window side converts it to tao's `CursorIcon`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    NwseResize,
    NeswResize,
    Move,
    Crosshair,
    Hand,
    Default,
}

/// The four corners of the area
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    /// The cursor above that corner
    fn cursor(self) -> Cursor {
        match self {
            Corner::TopLeft | Corner::BottomRight => Cursor::NwseResize,
            Corner::TopRight | Corner::BottomLeft => Cursor::NeswResize,
        }
    }
}

/// A key that was pressed (only passed when pressed)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
}

/// The result of an operation (the window side decides redrawing and closing)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Do nothing
    None,
    /// Redraw because the appearance changed
    Redraw,
    /// Finish the export with the area
    Export,
    /// Discard the export
    Cancel,
}

/// What a drag is doing
enum Drag {
    /// Change the size by moving a corner (the opposite corner does not move)
    Resize {
        corner: Corner,
        start: Rect,
        start_mouse: (f64, f64),
    },
    /// Move the area (do not change the size)
    Move {
        start: Rect,
        start_mouse: (f64, f64),
    },
    /// Draw a new area from where it was pressed (positions on the sheet)
    New { start: (f64, f64) },
}

/// What is under the mouse position
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    Button(Button),
    Corner(Corner),
    /// Inside the area
    Inside,
    /// On the sheet (outside the area, not a handle)
    SheetArea,
    /// Elsewhere (the margin, the bar, etc.)
    Elsewhere,
}

/// The state of the window operations (the area, drag, mouse position)
pub struct State {
    /// The size of the sheet (CSS pixels)
    pub page: PageSize,
    /// The current area (sheet CSS pixels)
    pub area: Rect,
    /// The layout of the screen
    pub layout: Layout,
    /// The mouse position (logical pixels)
    mouse: (f64, f64),
    /// The button that is pressed (it takes effect when released above the same button)
    pressed: Option<Button>,
    /// The drag in progress
    drag: Option<Drag>,
}

impl State {
    /// The initial state: the area is the whole sheet
    pub fn new(page: PageSize, width: f64, height: f64) -> Self {
        Self {
            page,
            area: Rect::from(page),
            layout: layout(page, width, height),
            mouse: (0.0, 0.0),
            pressed: None,
            drag: None,
        }
    }

    /// The size of the window changed (logical pixels)
    pub fn resize(&mut self, width: f64, height: f64) -> Outcome {
        self.layout = layout(self.page, width, height);
        Outcome::Redraw
    }

    /// The position of the mouse (logical pixels, it may be outside the window)
    pub fn move_mouse(&mut self, x: f64, y: f64) -> Outcome {
        let old = self.hovered_button();
        self.mouse = (x, y);
        if let Some(drag) = self.drag.take() {
            let mut drag = drag;
            self.drag_area(&mut drag, x, y);
            self.drag = Some(drag);
            return Outcome::Redraw;
        }
        // Redraw if the hovered button changed
        if self.hovered_button() != old {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    /// Press the mouse button (use the current mouse position)
    pub fn press(&mut self) -> Outcome {
        let (x, y) = self.mouse;
        match self.hit(x, y) {
            Hit::Button(button) => {
                self.pressed = Some(button);
                Outcome::Redraw
            }
            Hit::Corner(corner) => {
                self.drag = Some(Drag::Resize {
                    corner,
                    start: self.area,
                    start_mouse: (x, y),
                });
                Outcome::Redraw
            }
            Hit::Inside => {
                self.drag = Some(Drag::Move {
                    start: self.area,
                    start_mouse: (x, y),
                });
                Outcome::Redraw
            }
            Hit::SheetArea => {
                let start = self.to_sheet(x, y);
                self.area = self.new_area(start, start.0, start.1);
                self.drag = Some(Drag::New { start });
                Outcome::Redraw
            }
            Hit::Elsewhere => Outcome::None,
        }
    }

    /// Release the mouse button (use the current mouse position)
    pub fn release(&mut self) -> Outcome {
        let (x, y) = self.mouse;
        self.drag = None;
        // A button takes effect when pressed and released above the same button
        if let Some(pressed) = self.pressed.take()
            && self.hit(x, y) == Hit::Button(pressed)
        {
            return match pressed {
                Button::SelectAll => {
                    self.area = Rect::from(self.page);
                    Outcome::Redraw
                }
                Button::Cancel => Outcome::Cancel,
                Button::Export => Outcome::Export,
            };
        }
        Outcome::None
    }

    /// A key (when pressed)
    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Enter => Outcome::Export,
            Key::Escape => Outcome::Cancel,
        }
    }

    /// The shape of the cursor
    pub fn cursor(&self) -> Cursor {
        if let Some(drag) = &self.drag {
            return match drag {
                Drag::Resize { corner, .. } => corner.cursor(),
                Drag::Move { .. } => Cursor::Move,
                Drag::New { .. } => Cursor::Crosshair,
            };
        }
        match self.hit(self.mouse.0, self.mouse.1) {
            Hit::Button(_) => Cursor::Hand,
            Hit::Corner(corner) => corner.cursor(),
            Hit::Inside => Cursor::Move,
            Hit::SheetArea => Cursor::Crosshair,
            Hit::Elsewhere => Cursor::Default,
        }
    }

    /// The button that is being hovered (or pressed)
    pub fn hovered_button(&self) -> Option<Button> {
        if let Some(pressed) = self.pressed {
            return Some(pressed);
        }
        match self.hit(self.mouse.0, self.mouse.1) {
            Hit::Button(button) => Some(button),
            _ => None,
        }
    }

    /// What is under a position on the screen (logical pixels)
    fn hit(&self, x: f64, y: f64) -> Hit {
        for (button, region) in Button::ALL.iter().zip(&self.layout.buttons) {
            if region.contains(x, y) {
                return Hit::Button(*button);
            }
        }
        let sheet_area = self.layout.sheet_area;
        if sheet_area.w <= 0.0 || !sheet_area.contains(x, y) {
            return Hit::Elsewhere;
        }
        let (scale, area) = (self.layout.scale, self.area);
        // The four corners of the area (screen logical pixels)
        let corners = [
            (
                Corner::TopLeft,
                sheet_area.x + area.x * scale,
                sheet_area.y + area.y * scale,
            ),
            (
                Corner::TopRight,
                sheet_area.x + area.right() * scale,
                sheet_area.y + area.y * scale,
            ),
            (
                Corner::BottomLeft,
                sheet_area.x + area.x * scale,
                sheet_area.y + area.bottom() * scale,
            ),
            (
                Corner::BottomRight,
                sheet_area.x + area.right() * scale,
                sheet_area.y + area.bottom() * scale,
            ),
        ];
        let reach = HANDLE / 2.0;
        for (corner, cx, cy) in corners {
            if (x - cx).abs() <= reach && (y - cy).abs() <= reach {
                return Hit::Corner(corner);
            }
        }
        // The position on the sheet (CSS pixels)
        let (sx, sy) = ((x - sheet_area.x) / scale, (y - sheet_area.y) / scale);
        if area.x <= sx && sx <= area.right() && area.y <= sy && sy <= area.bottom() {
            Hit::Inside
        } else {
            Hit::SheetArea
        }
    }

    /// Turn a position on the screen (logical pixels) into a position on the sheet (CSS pixels) (positions outside the sheet are clamped to the edge of the sheet)
    fn to_sheet(&self, x: f64, y: f64) -> (f64, f64) {
        let sheet_area = self.layout.sheet_area;
        (
            ((x - sheet_area.x) / self.layout.scale).clamp(0.0, self.page.width),
            ((y - sheet_area.y) / self.layout.scale).clamp(0.0, self.page.height),
        )
    }

    /// A new area from start (a position on the sheet) to (sx, sy). Clamped inside the sheet, the width and height are at least 1 CSS pixel
    fn new_area(&self, start: (f64, f64), sx: f64, sy: f64) -> Rect {
        let left = start.0.min(sx).clamp(0.0, self.page.width);
        let right = start.0.max(sx).clamp(0.0, self.page.width);
        let top = start.1.min(sy).clamp(0.0, self.page.height);
        let bottom = start.1.max(sy).clamp(0.0, self.page.height);
        let (right, bottom) = (
            right.max(left + 1.0).min(self.page.width),
            bottom.max(top + 1.0).min(self.page.height),
        );
        Rect::from_edges(left, top, right, bottom)
    }

    /// Compute a new area from the position during a drag
    fn drag_area(&mut self, drag: &mut Drag, x: f64, y: f64) {
        let (scale, page) = (self.layout.scale, self.page);
        match drag {
            Drag::New { start } => {
                let (sx, sy) = self.to_sheet(x, y);
                self.area = self.new_area(*start, sx, sy);
            }
            Drag::Move { start, start_mouse } => {
                let (dx, dy) = ((x - start_mouse.0) / scale, (y - start_mouse.1) / scale);
                self.area = Rect {
                    x: (start.x + dx).clamp(0.0, page.width - start.width),
                    y: (start.y + dy).clamp(0.0, page.height - start.height),
                    width: start.width,
                    height: start.height,
                };
            }
            Drag::Resize {
                corner,
                start,
                start_mouse,
            } => {
                let (dx, dy) = ((x - start_mouse.0) / scale, (y - start_mouse.1) / scale);
                let (left, top) = (start.x, start.y);
                let (right, bottom) = (start.right(), start.bottom());
                // The opposite corner does not move. When it goes past the edge, clamp it to the edge of the sheet, and when it is less than 1, make it 1
                let (left, right) = match corner {
                    Corner::TopLeft | Corner::BottomLeft => {
                        ((left + dx).clamp(0.0, right - 1.0), right)
                    }
                    Corner::TopRight | Corner::BottomRight => {
                        (left, (right + dx).clamp(left + 1.0, page.width))
                    }
                };
                let (top, bottom) = match corner {
                    Corner::TopLeft | Corner::TopRight => {
                        ((top + dy).clamp(0.0, bottom - 1.0), bottom)
                    }
                    Corner::BottomLeft | Corner::BottomRight => {
                        (top, (bottom + dy).clamp(top + 1.0, page.height))
                    }
                };
                self.area = Rect::from_edges(left, top, right, bottom);
            }
        }
    }
}

/// The key that identifies the drawing of the sheet (the size of the sheet and the scale on the screen)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetRenderKey {
    /// The bit patterns of the width, height, and scale of the sheet
    width: u64,
    height: u64,
    scale: u64,
}

fn sheet_render_key(page: PageSize, scale: f64) -> SheetRenderKey {
    SheetRenderKey {
        width: page.width.to_bits(),
        height: page.height.to_bits(),
        scale: scale.to_bits(),
    }
}

/// The drawing of the sheet that is done on another thread (the pixels are not put in it, because they would be copied large)
#[derive(Debug, Clone, Copy)]
pub struct SheetRenderJob {
    /// The key that identifies this drawing (used to decide whether the result is still usable)
    pub key: SheetRenderKey,
    /// The size of the sheet (CSS pixels)
    pub page: PageSize,
    /// The physical pixels per 1 CSS pixel of the sheet
    pub scale: f64,
}

/// The content and operations of the window (independent of the window).
/// The strokes and the image are copies taken when the window was opened, and
/// they are not followed after that.
/// The pixels of the sheet are drawn on another thread (export_window.rs) and
/// passed in, and this holds only the result that matches the current layout
pub struct Scene {
    state: State,
    /// The strokes of the sheet (CSS pixels). Shared with the drawing thread via Arc
    strokes: Arc<Vec<Stroke>>,
    /// The image of the sheet (under the strokes)
    image: Option<(ImagePlacement, Arc<StoredImage>)>,
    /// The pixels of the sheet that match the current layout (physical pixels)
    sheet_render: Option<(SheetRenderKey, render::Pixels)>,
    /// The key of the drawing of the sheet in progress (does not start twice with the same key)
    drawing: Option<SheetRenderKey>,
}

impl Scene {
    /// Make one from the sheet when the window was opened (the area is the whole sheet)
    pub fn new(
        page: PageSize,
        strokes: Arc<Vec<Stroke>>,
        image: Option<(ImagePlacement, Arc<StoredImage>)>,
        width: f64,
        height: f64,
    ) -> Self {
        Self {
            state: State::new(page, width, height),
            strokes,
            image,
            sheet_render: None,
            drawing: None,
        }
    }

    /// The current area (CSS pixels)
    pub fn area(&self) -> Rect {
        self.state.area
    }

    pub fn resize(&mut self, width: f64, height: f64) -> Outcome {
        self.state.resize(width, height)
    }

    pub fn move_mouse(&mut self, x: f64, y: f64) -> Outcome {
        self.state.move_mouse(x, y)
    }

    pub fn press(&mut self) -> Outcome {
        self.state.press()
    }

    pub fn release(&mut self) -> Outcome {
        self.state.release()
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        self.state.key(key)
    }

    pub fn cursor(&self) -> Cursor {
        self.state.cursor()
    }

    pub fn hovered_button(&self) -> Option<Button> {
        self.state.hovered_button()
    }

    /// The key that identifies the drawing of the sheet for the current layout (the size of the sheet and the scale on the screen)
    fn current_sheet_render_key(&self, scale_factor: f64) -> SheetRenderKey {
        sheet_render_key(self.state.page, self.state.layout.scale * scale_factor)
    }

    /// The drawing of the sheet needed for the current layout. None if it already holds
    /// the result of a drawing that matches the current layout, or if a drawing with the same key is in progress
    pub fn sheet_render_job(&mut self, scale_factor: f64) -> Option<SheetRenderJob> {
        if self.state.layout.scale <= 0.0 {
            return None;
        }
        let key = self.current_sheet_render_key(scale_factor);
        if self
            .sheet_render
            .as_ref()
            .is_some_and(|(cached, _)| *cached == key)
            || self.drawing == Some(key)
        {
            return None;
        }
        self.drawing = Some(key);
        Some(SheetRenderJob {
            key,
            page: self.state.page,
            scale: self.state.layout.scale * scale_factor,
        })
    }

    /// Receive the pixels of the sheet drawn on another thread. If the key does not match
    /// the current layout, discard them (an old result from while the size was changing). Returns whether to redraw
    pub fn finish_sheet_render(
        &mut self,
        job: &SheetRenderJob,
        scale_factor: f64,
        pixels: render::Pixels,
    ) -> bool {
        if self.drawing == Some(job.key) {
            self.drawing = None;
        }
        if job.key != self.current_sheet_render_key(scale_factor) {
            return false;
        }
        self.sheet_render = Some((job.key, pixels));
        true
    }

    /// The strokes of the drawing of the sheet (not copied, because they are shared via Arc)
    pub fn sheet_strokes(&self) -> &Arc<Vec<Stroke>> {
        &self.strokes
    }

    /// The image of the drawing of the sheet (not copied, because it is shared via Arc)
    pub fn sheet_image(&self) -> Option<(ImagePlacement, Arc<StoredImage>)> {
        self.image.clone()
    }

    /// The pixels of the sheet that match the current layout (None if not held, or if they do not match)
    fn sheet_render_pixels(&self, scale_factor: f64) -> Option<&render::Pixels> {
        self.sheet_render
            .as_ref()
            .filter(|(key, _)| *key == self.current_sheet_render_key(scale_factor))
            .map(|(_, pixels)| pixels)
    }

    /// Draw the whole screen (physical pixels, u32 as `0x00RRGGBB`)
    pub fn paint(&mut self, width: u32, height: u32, scale_factor: f64) -> Vec<u32> {
        let (layout, area) = (self.state.layout, self.state.area);
        let mut pixels = vec![BACKGROUND; (width * height) as usize];
        if width == 0 || height == 0 {
            return pixels;
        }
        // The physical pixels per 1 CSS pixel
        let total = layout.scale * scale_factor;

        // The sheet (outside the area is darkened). The pixels are drawn on another thread
        // and passed in; if they have not arrived, fill the place of the sheet with white
        if layout.scale > 0.0 {
            let (px0, py0) = (
                (layout.sheet_area.x * scale_factor).round() as i64,
                (layout.sheet_area.y * scale_factor).round() as i64,
            );
            // The edges of the area (window physical pixels)
            let (rx0, ry0) = (
                px0 + (area.x * total).round() as i64,
                py0 + (area.y * total).round() as i64,
            );
            let (rx1, ry1) = (
                px0 + (area.right() * total).round() as i64,
                py0 + (area.bottom() * total).round() as i64,
            );
            if let Some(sheet_render_pixels) = self.sheet_render_pixels(scale_factor) {
                for (y, row) in sheet_render_pixels
                    .rgb
                    .chunks(sheet_render_pixels.width as usize * 3)
                    .enumerate()
                {
                    let wy = py0 + y as i64;
                    if wy < 0 || wy >= height as i64 {
                        continue;
                    }
                    for (x, rgb) in row.chunks(3).enumerate() {
                        let wx = px0 + x as i64;
                        if wx < 0 || wx >= width as i64 {
                            continue;
                        }
                        let color = if rx0 <= wx && wx < rx1 && ry0 <= wy && wy < ry1 {
                            to_u32(rgb)
                        } else {
                            darken(to_u32(rgb))
                        };
                        pixels[wy as usize * width as usize + wx as usize] = color;
                    }
                }
            } else {
                // There is no result of the drawing yet: the sheet is white, and outside the area is darkened (the same color as the copy)
                let (pw, ph) = (
                    (layout.sheet_area.w * scale_factor).round() as i64,
                    (layout.sheet_area.h * scale_factor).round() as i64,
                );
                let dimmed = darken(WHITE);
                fill_rect(&mut pixels, width, px0, py0, pw, ph, dimmed);
                fill_rect(&mut pixels, width, rx0, ry0, rx1 - rx0, ry1 - ry0, WHITE);
            }
            // The border of the area (2 physical pixels inside the sheet)
            let border = BORDER as i64;
            fill_rect(&mut pixels, width, rx0, ry0, rx1 - rx0, border, BLUE);
            fill_rect(
                &mut pixels,
                width,
                rx0,
                ry1 - border,
                rx1 - rx0,
                border,
                BLUE,
            );
            fill_rect(&mut pixels, width, rx0, ry0, border, ry1 - ry0, BLUE);
            fill_rect(
                &mut pixels,
                width,
                rx1 - border,
                ry0,
                border,
                ry1 - ry0,
                BLUE,
            );
            // The handles at the four corners (10 logical pixels on each side. A white fill with a blue edge)
            let size = (HANDLE * scale_factor).round() as i64;
            for (cx, cy) in [(rx0, ry0), (rx1, ry0), (rx0, ry1), (rx1, ry1)] {
                let (hx, hy) = (cx - size / 2, cy - size / 2);
                fill_rect(&mut pixels, width, hx, hy, size, size, WHITE);
                fill_rect(&mut pixels, width, hx, hy, size, border, BLUE);
                fill_rect(
                    &mut pixels,
                    width,
                    hx,
                    hy + size - border,
                    size,
                    border,
                    BLUE,
                );
                fill_rect(&mut pixels, width, hx, hy, border, size, BLUE);
                fill_rect(
                    &mut pixels,
                    width,
                    hx + size - border,
                    hy,
                    border,
                    size,
                    BLUE,
                );
            }
        }

        // The bar at the bottom (a white fill, a 1 physical pixel gray line on the top edge)
        let band_top = (height as f64 - BAND_HEIGHT * scale_factor).round() as i64;
        fill_rect(
            &mut pixels,
            width,
            0,
            band_top,
            width as i64,
            height as i64 - band_top,
            WHITE,
        );
        fill_rect(&mut pixels, width, 0, band_top, width as i64, 1, LINE_GRAY);

        // The left of the bar: the size of the area (CSS pixels)
        let label = format!(
            "{} x {}",
            (area.width).round() as i64,
            (area.height).round() as i64
        );
        let text = text_strokes(&label, 0.0, 0.0, FONT_SIZE);
        let Some((min_x, min_y, max_x, max_y)) = ink_bounds(&text) else {
            return pixels;
        };
        let (left, top) = (min_x.floor(), min_y.floor());
        let (w, h) = (
            (max_x - left).ceil().max(1.0),
            (max_y - top).ceil().max(1.0),
        );
        let strokes: Vec<Stroke> = text
            .iter()
            .cloned()
            .map(|points| window_stroke(points, TEXT))
            .collect();
        let drawn = render::render_area_on(
            [255, 255, 255],
            &strokes,
            None,
            Rect::from_edges(left, top, left + w, top + h),
            scale_factor,
        );
        blit(
            &mut pixels,
            width,
            &drawn,
            ((layout.size_text.0 - left) * scale_factor).round() as i64,
            ((layout.size_text.1 - top) * scale_factor).round() as i64,
        );

        // The right of the bar: draw each button and blit it onto the bar
        for (button, region) in Button::ALL.iter().zip(&layout.buttons) {
            let background = if self.hovered_button() == Some(*button) {
                rgb8(HOVER)
            } else {
                rgb8(WHITE)
            };
            // The border (a rectangular border. Moved inside by the width of the line so it is not clipped)
            let line_color = match button {
                Button::Export => BLUE,
                _ => LINE_GRAY,
            };
            let mut strokes = shape_strokes(
                ShapeKind::Rect {
                    x: 0.75,
                    y: 0.75,
                    width: region.w - 1.5,
                    height: region.h - 1.5,
                },
                line_color,
            );
            // The text (centered in the button. Export is blue)
            let ink = text_ink_width(button.label(), FONT_SIZE);
            strokes.extend(
                text_strokes(
                    button.label(),
                    (region.w - ink) / 2.0,
                    (region.h - FONT_SIZE) / 2.0,
                    FONT_SIZE,
                )
                .into_iter()
                .map(|points| {
                    let color = match button {
                        Button::Export => BLUE,
                        _ => TEXT,
                    };
                    window_stroke(points, color)
                }),
            );
            let drawn = render::render_area_on(
                background,
                &strokes,
                None,
                Rect::from_edges(0.0, 0.0, region.w, region.h),
                scale_factor,
            );
            blit(
                &mut pixels,
                width,
                &drawn,
                (region.x * scale_factor).round() as i64,
                (region.y * scale_factor).round() as i64,
            );
        }
        pixels
    }
}

/// The ink bounds of the points (logical pixels). None if there are no points
fn ink_bounds(strokes: &[Vec<StrokePoint>]) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in strokes.iter().flatten() {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if min_x.is_infinite() {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

/// Blit drawn pixels (RGB) onto the screen (the position may go outside the screen)
fn blit(dest: &mut [u32], dest_width: u32, source: &render::Pixels, x: i64, y: i64) {
    if dest_width == 0 {
        return;
    }
    let dest_height = dest.len() as i64 / dest_width as i64;
    for (sy, row) in source.rgb.chunks(source.width as usize * 3).enumerate() {
        let dy = y + sy as i64;
        if dy < 0 || dy >= dest_height {
            continue;
        }
        for (sx, rgb) in row.chunks(3).enumerate() {
            let dx = x + sx as i64;
            if dx < 0 || dx >= dest_width as i64 {
                continue;
            }
            dest[dy as usize * dest_width as usize + dx as usize] = to_u32(rgb);
        }
    }
}

/// Fill a rectangle (the position may go outside the screen)
fn fill_rect(pixels: &mut [u32], width: u32, x: i64, y: i64, w: i64, h: i64, color: u32) {
    if width == 0 || w <= 0 || h <= 0 {
        return;
    }
    let height = pixels.len() as i64 / width as i64;
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(width as i64), (y + h).min(height));
    for dy in y0.max(0)..y1 {
        for dx in x0.max(0)..x1 {
            pixels[dy as usize * width as usize + dx as usize] = color;
        }
    }
}

/// Turn a screen pixel (0x00RRGGBB) into RGB (0 to 255)
fn rgb8(color: u32) -> [u8; 3] {
    [
        (color >> 16 & 0xff) as u8,
        (color >> 8 & 0xff) as u8,
        (color & 0xff) as u8,
    ]
}

/// Turn RGB (0 to 255) into a screen pixel
fn to_u32(rgb: &[u8]) -> u32 {
    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}

/// Darken by overlaying 40% black
fn darken(pixel: u32) -> u32 {
    let (r, g, b) = (
        ((pixel >> 16 & 0xff) as f32 * 0.6).round(),
        ((pixel >> 8 & 0xff) as f32 * 0.6).round(),
        ((pixel & 0xff) as f32 * 0.6).round(),
    );
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: f64, height: f64) -> PageSize {
        PageSize { width, height }
    }

    /// The window (logical 480×360), the sheet 400×300
    fn state() -> State {
        State::new(page(400.0, 300.0), 480.0, 360.0)
    }

    fn sheet_area_edges(state: &State) -> (f64, f64, f64, f64) {
        let sheet_area = state.layout.sheet_area;
        (
            sheet_area.x,
            sheet_area.y,
            sheet_area.x + sheet_area.w,
            sheet_area.y + sheet_area.h,
        )
    }

    #[test]
    fn the_layout_centers_the_sheet_in_the_usable_area() {
        let usable_w = 960.0 - PADDING * 2.0;
        let usable_h = 720.0 - BAND_HEIGHT - PADDING * 2.0;
        // A landscape sheet: the width limits it
        let landscape = layout(page(800.0, 500.0), 960.0, 720.0);
        assert!(
            (landscape.scale - usable_w / 800.0).abs() < 1e-9,
            "幅で収まる"
        );
        assert!((landscape.sheet_area.w - usable_w).abs() < 1e-9);
        assert!((landscape.sheet_area.x - PADDING).abs() < 1e-9, "横に中央");
        assert!(
            (landscape.sheet_area.y - (720.0 - BAND_HEIGHT - landscape.sheet_area.h) / 2.0).abs()
                < 1e-9,
            "縦に中央"
        );

        // A portrait sheet: the height limits it
        let portrait = layout(page(600.0, 800.0), 960.0, 720.0);
        assert!(
            (portrait.scale - usable_h / 800.0).abs() < 1e-9,
            "高さで収まる"
        );
        assert!((portrait.sheet_area.h - usable_h).abs() < 1e-9);
        assert!((portrait.sheet_area.y - PADDING).abs() < 1e-9, "縦に中央");
        assert!(
            (portrait.sheet_area.x - (960.0 - portrait.sheet_area.w) / 2.0).abs() < 1e-9,
            "横に中央"
        );
    }

    #[test]
    fn the_buttons_are_right_aligned_in_the_band() {
        let layout = layout(page(800.0, 500.0), 960.0, 720.0);
        let band_top = 720.0 - BAND_HEIGHT;
        // Right-aligned, and vertically centered in the bar
        let export = layout.buttons[2];
        assert!(
            (export.x + export.w - (960.0 - PADDING)).abs() < 1e-9,
            "右詰め"
        );
        assert!((export.y - (band_top + (BAND_HEIGHT - BUTTON_HEIGHT) / 2.0)).abs() < 1e-9);
        // From left to right, with a gap between them
        for pair in layout.buttons.windows(2) {
            assert!((pair[0].x + pair[0].w + BUTTON_GAP - pair[1].x).abs() < 1e-9);
            assert_eq!(pair[1].h, BUTTON_HEIGHT);
        }
    }

    #[test]
    fn hit_testing_reports_corners_inside_sheet_and_buttons() {
        let state = state();
        let (left, top, right, bottom) = sheet_area_edges(&state);
        // The four corners (the initial area is the whole sheet, so the four corners of the sheet)
        assert_eq!(state.hit(left, top), Hit::Corner(Corner::TopLeft));
        assert_eq!(state.hit(right, top), Hit::Corner(Corner::TopRight));
        assert_eq!(state.hit(left, bottom), Hit::Corner(Corner::BottomLeft));
        assert_eq!(state.hit(right, bottom), Hit::Corner(Corner::BottomRight));
        // Inside the area
        assert_eq!(
            state.hit((left + right) / 2.0, (top + bottom) / 2.0),
            Hit::Inside
        );
        // A button
        let export = state.layout.buttons[2];
        assert_eq!(
            state.hit(export.x + export.w / 2.0, export.y + export.h / 2.0),
            Hit::Button(Button::Export)
        );
        // The margin
        assert_eq!(state.hit(0.0, 0.0), Hit::Elsewhere);
    }

    #[test]
    fn the_cursor_follows_the_mouse_position() {
        let mut state = state();
        let (left, top, right, bottom) = sheet_area_edges(&state);
        state.move_mouse((left + right) / 2.0, (top + bottom) / 2.0);
        assert_eq!(state.cursor(), Cursor::Move);
        state.move_mouse(left, top);
        assert_eq!(state.cursor(), Cursor::NwseResize);
        state.move_mouse(right, top);
        assert_eq!(state.cursor(), Cursor::NeswResize);
        // After shrinking the area, on the sheet (outside the area) it is Crosshair
        state.press(); // The TopLeft handle
        state.move_mouse(left + 100.0, top + 100.0);
        state.release();
        state.move_mouse(left + 10.0, top + 10.0);
        assert_eq!(state.cursor(), Cursor::Crosshair);
        // A button
        let export = state.layout.buttons[2];
        state.move_mouse(export.x + 1.0, export.y + 1.0);
        assert_eq!(state.cursor(), Cursor::Hand);
        // The margin
        state.move_mouse(0.0, 0.0);
        assert_eq!(state.cursor(), Cursor::Default);
    }

    /// When dragging a corner, the opposite corner does not move, and the area does not go past the sheet or become less than 1
    #[test]
    fn resizing_from_a_corner_keeps_the_opposite_corner_and_clamps() {
        let mut state = state();
        let (left, top, right, bottom) = sheet_area_edges(&state);
        // Drag the BottomRight handle to the bottom right, past the sheet
        state.move_mouse(right, bottom);
        state.press();
        state.move_mouse(right + 100.0, bottom + 100.0);
        assert_eq!(state.area.right(), 400.0, "シートの右に収める");
        assert_eq!(state.area.bottom(), 300.0, "シートの下に収める");
        assert_eq!(state.area.x, 0.0, "向かいの隅は動かない");
        assert_eq!(state.area.y, 0.0);
        // Even when dragging on to the top left corner, the area does not flip and the minimum is 1
        state.move_mouse(left - 50.0, top - 50.0);
        assert_eq!(state.area.width, 1.0, "幅の最小は 1");
        assert_eq!(state.area.height, 1.0, "高さの最小は 1");
    }

    /// A drag inside the area moves while keeping the size, and does not go outside the sheet
    #[test]
    fn moving_the_range_keeps_the_size_and_stays_on_the_sheet() {
        let mut state = state();
        let (left, top, right, bottom) = sheet_area_edges(&state);
        // First shrink the area (drag the BottomRight handle to the center)
        state.move_mouse(right, bottom);
        state.press();
        state.move_mouse((left + right) / 2.0, (top + bottom) / 2.0);
        state.release();
        let (width, height) = (state.area.width, state.area.height);
        // Drag inside the area ((50, 50) on the sheet) to the bottom right, past the sheet
        let (scale, sheet_area) = (state.layout.scale, state.layout.sheet_area);
        state.move_mouse(sheet_area.x + 50.0 * scale, sheet_area.y + 50.0 * scale);
        state.press();
        state.move_mouse(10000.0, 10000.0);
        assert_eq!(state.area.width, width, "大きさは変えない");
        assert_eq!(state.area.height, height);
        assert!(
            (state.area.x - (400.0 - width)).abs() < 1e-9,
            "シートの右に収める"
        );
        assert!(
            (state.area.y - (300.0 - height)).abs() < 1e-9,
            "シートの下に収める"
        );
    }

    /// A drag outside the area (on the sheet) draws a new area from where it was pressed
    #[test]
    fn pressing_outside_the_range_starts_a_new_one() {
        let mut state = state();
        let (left, top, _right, _bottom) = sheet_area_edges(&state);
        let scale = state.layout.scale;
        // First shrink the area to the top right (drag the TopLeft handle to the center)
        state.move_mouse(left, top);
        state.press();
        state.move_mouse(left + 100.0, top + 100.0);
        state.release();
        assert!(state.area.x > 0.0, "範囲は縮まった");
        // Press outside the area (top left) and drag to the bottom right
        state.move_mouse(left + 10.0, top + 10.0);
        state.press();
        state.move_mouse(left + 100.0, top + 100.0);
        assert!((state.area.x - 10.0 / scale).abs() < 1e-9);
        assert!(
            (state.area.width - 90.0 / scale).abs() < 1e-9,
            "押した所から描く"
        );
        // If pressed without moving, the width and height are 1 (a position that does not rest on the corner handle of the area)
        state.release();
        state.move_mouse(left + 2.0, top + 2.0);
        state.press();
        state.release();
        assert_eq!(state.area.width, 1.0);
        assert_eq!(state.area.height, 1.0);
    }

    /// Select All returns the area to the whole sheet
    #[test]
    fn select_all_restores_the_whole_page() {
        let mut state = state();
        let (left, _top, right, bottom) = sheet_area_edges(&state);
        // Shrink the area
        state.move_mouse(right, bottom);
        state.press();
        state.move_mouse(left + 100.0, bottom);
        state.release();
        assert!(state.area.width < 400.0);
        // Press Select All and release above the same button
        let button = state.layout.buttons[0];
        state.move_mouse(button.x + 1.0, button.y + 1.0);
        state.press();
        state.move_mouse(button.x + 2.0, button.y + 2.0);
        assert_eq!(state.release(), Outcome::Redraw);
        assert_eq!(state.area, Rect::from(page(400.0, 300.0)));
    }

    /// A button takes effect when pressed and released above the same button
    #[test]
    fn the_buttons_fire_only_when_released_on_themselves() {
        let mut state = state();
        let cancel = state.layout.buttons[1];
        state.move_mouse(cancel.x + 1.0, cancel.y + 1.0);
        state.press();
        state.move_mouse(cancel.x + 2.0, cancel.y + 2.0);
        assert_eq!(state.release(), Outcome::Cancel);

        // It does not take effect when released somewhere else
        state.move_mouse(cancel.x + 1.0, cancel.y + 1.0);
        state.press();
        state.move_mouse(0.0, 0.0);
        assert_eq!(state.release(), Outcome::None);

        // Export
        let export = state.layout.buttons[2];
        state.move_mouse(export.x + 1.0, export.y + 1.0);
        state.press();
        state.move_mouse(export.x + 2.0, export.y + 2.0);
        assert_eq!(state.release(), Outcome::Export);
    }

    /// Enter is Export, Esc is Cancel
    #[test]
    fn enter_exports_and_escape_cancels() {
        let mut state = state();
        assert_eq!(state.key(Key::Enter), Outcome::Export);
        assert_eq!(state.key(Key::Escape), Outcome::Cancel);
    }

    /// A scene with a white sheet that has no strokes (the sizes are logical pixels)
    fn scene(width: f64, height: f64) -> Scene {
        Scene::new(
            page(400.0, 300.0),
            Arc::new(Vec::new()),
            None,
            width,
            height,
        )
    }

    /// count strokes (in the form shared via Arc)
    fn strokes(count: usize) -> Arc<Vec<Stroke>> {
        Arc::new(
            (0..count)
                .map(|index| Stroke {
                    id: format!("stroke-{index}"),
                    author: Author::Human,
                    color: "#000000".into(),
                    points: vec![
                        StrokePoint {
                            x: 10.0,
                            y: 10.0,
                            pressure: 0.5,
                        },
                        StrokePoint {
                            x: 20.0,
                            y: 20.0,
                            pressure: 0.5,
                        },
                    ],
                })
                .collect(),
        )
    }

    /// The colors of the margin, the bar, and the border (scale_factor 1 and 2)
    #[test]
    fn paint_fills_margin_band_and_border() {
        for scale_factor in [1.0, 2.0] {
            let (width, height) = ((480.0 * scale_factor) as u32, (360.0 * scale_factor) as u32);
            let mut scene = scene(480.0, 360.0);
            let pixels = scene.paint(width, height, scale_factor);
            let at = |x: u32, y: u32| pixels[y as usize * width as usize + x as usize];

            // The margin is light gray
            assert_eq!(at(0, 0), BACKGROUND);
            // The bar is white, with a 1 physical pixel gray line on the top edge
            let band_top = (360.0 * scale_factor - BAND_HEIGHT * scale_factor).round() as u32;
            assert_eq!(at(0, band_top), LINE_GRAY);
            assert_eq!(at(0, band_top + 1), WHITE);
            // The sheet is white (no strokes)
            let (sheet_x, sheet_y) = (
                (scene.state.layout.sheet_area.x * scale_factor).round() as u32,
                (scene.state.layout.sheet_area.y * scale_factor).round() as u32,
            );
            assert_eq!(at(sheet_x + 5, sheet_y + 5), WHITE);
            // The border of the area (initially the whole sheet) is blue, 2 physical pixels
            // wide (the middle of the top edge. The corners of the sheet are avoided, because the handles are on top of them)
            for offset in 0..BORDER {
                assert_eq!(at(sheet_x + 50, sheet_y + offset), BLUE, "枠の上は青");
            }
            assert_eq!(at(sheet_x + 50, sheet_y + BORDER), WHITE, "枠より内側は白");
        }
    }

    /// Outside the area (on the sheet) is darkened with 40% black, and inside the area is not darkened
    #[test]
    fn paint_darkens_the_sheet_outside_the_range() {
        let mut scene = scene(480.0, 360.0);
        let (left, top, right, bottom) = (
            scene.state.layout.sheet_area.x,
            scene.state.layout.sheet_area.y,
            scene.state.layout.sheet_area.x + scene.state.layout.sheet_area.w,
            scene.state.layout.sheet_area.y + scene.state.layout.sheet_area.h,
        );
        // Shrink the area to the center (drag the BottomRight handle)
        scene.move_mouse(right, bottom);
        scene.press();
        scene.move_mouse((left + right) / 2.0, (top + bottom) / 2.0);

        let width = 480u32;
        let pixels = scene.paint(width, 360, 1.0);
        let at = |x: u32, y: u32| pixels[y as usize * width as usize + x as usize];
        let (sheet_x, sheet_y) = (
            (scene.state.layout.sheet_area.x * 1.0).round() as u32,
            (scene.state.layout.sheet_area.y * 1.0).round() as u32,
        );
        // The top right of the sheet (outside the area) is white darkened with 40% black
        assert_eq!(at(300, 200), 0x0099_9999, "範囲の外は暗い");
        // Inside the area (toward the top left of the sheet) is not darkened
        assert_eq!(at(140, 80), WHITE, "範囲の中は暗くしない");
        // The corner handle (a white fill with a blue edge) reaches into the margin
        assert_eq!(at(sheet_x - 4, sheet_y - 4), BLUE, "ハンドルの縁は青");
        assert_eq!(at(sheet_x - 1, sheet_y + 1), WHITE, "ハンドルの地は白");
    }

    /// Immediately after opening, a drawing of the sheet is requested, and until the result arrives,
    /// paint fills the place of the sheet with white (paint does not draw the sheet)
    #[test]
    fn on_open_a_sheet_render_is_requested_and_paint_fills_the_sheet_white() {
        let mut scene = Scene::new(page(400.0, 300.0), strokes(3), None, 480.0, 360.0);
        assert!(
            scene.sheet_render_job(1.0).is_some(),
            "開いた直後はシートの描画を求める"
        );
        // Do not start a drawing with the same key twice
        assert!(
            scene.sheet_render_job(1.0).is_none(),
            "同じキーは二重に始めない"
        );
        // When drawn before the result arrives, the place of the sheet is filled with white (the border, handles, and bar are drawn)
        let width = 480u32;
        let pixels = scene.paint(width, 360, 1.0);
        let at = |x: u32, y: u32| pixels[y as usize * width as usize + x as usize];
        let sheet_area = scene.state.layout.sheet_area;
        let (sheet_x, sheet_y) = ((sheet_area.x).round() as u32, (sheet_area.y).round() as u32);
        assert_eq!(at(sheet_x + 50, sheet_y + 50), WHITE, "シートの場所は白");
        assert_eq!(at(sheet_x + 50, sheet_y), BLUE, "枠は描く");
    }

    /// When passed the result of the requested drawing, paint blits those pixels
    #[test]
    fn the_fetched_sheet_render_is_blitted_by_paint() {
        let mut scene = Scene::new(page(400.0, 300.0), strokes(3), None, 480.0, 360.0);
        let job = scene.sheet_render_job(1.0).expect("描画を求める");
        // A single-color result of the same size as the drawing
        let size = render::render_area(&[], None, Rect::from(job.page), job.scale);
        let fetched = render::Pixels {
            width: size.width,
            height: size.height,
            rgb: vec![0x12; (size.width as usize) * (size.height as usize) * 3],
        };
        assert!(
            scene.finish_sheet_render(&job, 1.0, fetched),
            "合う結果は使う"
        );
        let width = 480u32;
        let pixels = scene.paint(width, 360, 1.0);
        let at = |x: u32, y: u32| pixels[y as usize * width as usize + x as usize];
        let sheet_area = scene.state.layout.sheet_area;
        let (sheet_x, sheet_y) = ((sheet_area.x).round() as u32, (sheet_area.y).round() as u32);
        assert_eq!(
            at(sheet_x + 50, sheet_y + 50),
            0x0012_1212,
            "結果の画素を写す"
        );
        // Since it holds the matching result, the same drawing is not requested
        assert!(scene.sheet_render_job(1.0).is_none(), "描画し直さない");
    }

    /// When the size changes, a drawing with a new key is requested, and the result of the old key is discarded
    #[test]
    fn a_resize_requests_a_new_key_and_discards_the_stale_result() {
        let mut scene = Scene::new(page(400.0, 300.0), strokes(3), None, 480.0, 360.0);
        let old = scene.sheet_render_job(1.0).expect("描画を求める");
        // Change the size → the layout changes → request a drawing with a new key
        scene.resize(960.0, 720.0);
        let new = scene
            .sheet_render_job(1.0)
            .expect("新しいキーの描画を求める");
        assert_ne!(old.key, new.key, "キーは変わる");
        // The result of the old key is discarded (and it does not redraw)
        let stale = render::Pixels {
            width: 1,
            height: 1,
            rgb: vec![0x99, 0x88, 0x77],
        };
        assert!(
            !scene.finish_sheet_render(&old, 1.0, stale),
            "古いキーの結果は捨てる"
        );
        // Since it is discarded, it is still white
        let width = 960u32;
        let pixels = scene.paint(width, 720, 1.0);
        let sheet_area = scene.state.layout.sheet_area;
        let (sheet_x, sheet_y) = ((sheet_area.x).round() as u32, (sheet_area.y).round() as u32);
        assert_eq!(
            pixels[(sheet_y + 50) as usize * width as usize + (sheet_x + 50) as usize],
            WHITE,
            "古い結果は使わない"
        );
        // The result of the new key is used
        let size = render::render_area(&[], None, Rect::from(new.page), new.scale);
        let fetched = render::Pixels {
            width: size.width,
            height: size.height,
            rgb: vec![0x65; (size.width as usize) * (size.height as usize) * 3],
        };
        assert!(
            scene.finish_sheet_render(&new, 1.0, fetched),
            "新しいキーの結果は使う"
        );
    }

    /// Even with a sheet of 30,000 strokes, paint and the mouse and size operations
    /// return without drawing the sheet (the drawing of the sheet is done only outside the Scene (on another thread in export_window.rs))
    #[test]
    fn a_dense_sheet_does_not_render_inside_the_handlers() {
        let mut scene = Scene::new(page(400.0, 300.0), strokes(30000), None, 480.0, 360.0);
        assert!(scene.sheet_render_job(1.0).is_some(), "描画を求める");
        let sheet_area = scene.state.layout.sheet_area;
        // The operations without a drawing and paint return
        scene.move_mouse(sheet_area.x + 10.0, sheet_area.y + 10.0);
        scene.press();
        scene.move_mouse(sheet_area.x + 100.0, sheet_area.y + 100.0);
        scene.release();
        scene.resize(960.0, 720.0);
        assert!(
            scene.sheet_render_job(1.0).is_some(),
            "大きさを変えると描画を求める"
        );
        let width = 960u32;
        let pixels = scene.paint(width, 720, 1.0);
        assert_eq!(pixels.len(), (width * 720) as usize);
        // Since there is no result yet, the place of the sheet is white
        let sheet_area = scene.state.layout.sheet_area;
        let (sheet_x, sheet_y) = ((sheet_area.x).round() as u32, (sheet_area.y).round() as u32);
        assert_eq!(
            pixels[(sheet_y + 50) as usize * width as usize + (sheet_x + 50) as usize],
            WHITE,
            "シートは白で塗るだけ"
        );
    }
}
