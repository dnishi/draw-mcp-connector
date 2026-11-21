//! Menu bar (macOS) / notification area (Windows) icon, and the QR code window.
//! The app's only UI is the menu (connection status, "Show QR Code", "Open Canvas", "Export", "About", "Quit";
//! a connected status line has a "Disconnect" submenu, and "Export" has a "PNG" / "PDF" submenu), the QR code window,
//! the export area window (export_window.rs),
//! and the connection approval dialog (approval_dialog.rs).
//! On macOS the event loop and icon must run on the main thread (per the tray-icon documentation).

use std::sync::Arc;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget};
use tao::window::{Theme, Window, WindowBuilder, WindowId};
use tracing::warn;
use tray_icon::menu::{
    Icon as MenuIcon, IconMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem,
    PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::app::App;
use crate::area_select::{Outcome, SheetRenderJob};
use crate::config;
use crate::export::Format;
use crate::export_flow;
use crate::export_window::ExportWindow;
use crate::icon::{self, Appearance, Badge};
use crate::link_status::LinkStatus;
use crate::qr_window::QrWindow;
use crate::render::{self, Rect};

/// Events delivered to the event loop
pub(crate) enum UserEvent {
    Menu(MenuEvent),
    /// Open the QR code window from another thread (MCP's connect_canvas, the first-run guide)
    ShowQr,
    /// The connection state with the canvas changed (read the current state from app.status)
    Status,
    /// The QR contents (address or token) changed (redraws an open QR window)
    Pairing,
    /// The export flow asked for an area (open the area window, and call finish when it is decided)
    ExportArea(export_flow::AreaRequest),
    /// The sheet of the area window was drawn on another thread (the window id and the result)
    SheetRendered(WindowId, SheetRenderJob, render::Pixels),
}

fn badge(status: LinkStatus) -> Badge {
    match status {
        LinkStatus::Disconnected => Badge::None,
        LinkStatus::Waiting(_) => Badge::Waiting,
        LinkStatus::Connected(_) => Badge::Connected,
    }
}

fn status_text(status: LinkStatus) -> String {
    match status {
        LinkStatus::Disconnected => "Canvas: Not Connected".into(),
        LinkStatus::Waiting(ip) => format!("Canvas: Waiting for Approval ({ip})"),
        LinkStatus::Connected(ip) => format!("Canvas: Connected ({ip})"),
    }
}

/// Status lines of the menu. One line per connection (in connection order). A single Disconnected line
/// (with no number) if there are none
fn status_rows(links: &[(u64, LinkStatus)]) -> Vec<(Option<u64>, LinkStatus)> {
    if links.is_empty() {
        return vec![(None, LinkStatus::Disconnected)];
    }
    links
        .iter()
        .map(|(id, status)| (Some(*id), *status))
        .collect()
}

/// The dot at the start of a status line. Same color as the icon's dot; no dot when not connected
fn status_dot(status: LinkStatus, appearance: Appearance) -> Option<MenuIcon> {
    let rgba = icon::dot_rgba(badge(status), appearance)?;
    let dot = MenuIcon::from_rgba(rgba, icon::DOT_SIZE, icon::DOT_SIZE);
    Some(dot.expect("丸の大きさは正しい"))
}

/// A menu item for a status line. A connected line is enabled and has a "Disconnect" submenu;
/// other lines are disabled
enum StatusItem {
    Connected {
        row: Submenu,
        /// The "Disconnect" item in the submenu
        disconnect: MenuItem,
        /// Number of the connection this line shows
        connection: u64,
    },
    /// Pressing it would do nothing, so it is disabled (shown in faint text on macOS)
    Plain(IconMenuItem),
}

impl StatusItem {
    /// The item added to and removed from the menu (the submenu line when connected)
    fn menu_item(&self) -> &dyn IsMenuItem {
        match self {
            Self::Connected { row, .. } => row,
            Self::Plain(item) => item,
        }
    }

    /// The connection number if the pressed item's id is this line's "Disconnect"
    fn disconnect_connection(&self, id: &MenuId) -> Option<u64> {
        match self {
            Self::Connected {
                disconnect,
                connection,
                ..
            } if *id == *disconnect.id() => Some(*connection),
            _ => None,
        }
    }

    fn set_status(&self, status: LinkStatus, appearance: Appearance) {
        let text = status_text(status);
        let dot = status_dot(status, appearance);
        match self {
            Self::Connected { row, .. } => {
                row.set_text(text);
                row.set_icon(dot);
            }
            Self::Plain(item) => {
                item.set_text(text);
                item.set_icon(dot);
            }
        }
    }
}

/// Creates a status line. A connected connection becomes a submenu holding a single "Disconnect";
/// anything else is a disabled item
fn status_row(id: Option<u64>, status: LinkStatus) -> StatusItem {
    if let Some(connection) = id
        && matches!(status, LinkStatus::Connected(_))
    {
        let row = Submenu::new("", true);
        let disconnect = MenuItem::new("Disconnect", true, None);
        if let Err(error) = row.append(&disconnect) {
            warn!("Disconnect の項目を足せない: {error}");
        }
        return StatusItem::Connected {
            row,
            disconnect,
            connection,
        };
    }
    StatusItem::Plain(IconMenuItem::new("", false, None, None))
}

fn appearance(theme: Theme) -> Appearance {
    match theme {
        Theme::Dark => Appearance::Dark,
        // Theme may gain variants (non_exhaustive). Treat unknown ones as light
        _ => Appearance::Light,
    }
}

fn icon(status: LinkStatus, appearance: Appearance) -> Icon {
    Icon::from_rgba(
        icon::rgba(badge(status), appearance),
        icon::SIZE,
        icon::SIZE,
    )
    .expect("アイコンの大きさは正しい")
}

/// What the menu bar shows (icon, tooltip, and the menu's status lines)
struct Indicator {
    tray: TrayIcon,
    menu: Menu,
    /// Status lines placed at the top of the menu
    status_items: Vec<StatusItem>,
    /// Layout of the lines (connection number, whether connected), used to decide when to recreate the items
    row_kinds: Vec<(Option<u64>, bool)>,
    /// State to display, used for the icon's dot color (the state of the connection whose state changed last)
    status: LinkStatus,
    appearance: Appearance,
}

impl Indicator {
    /// status sets the icon's dot color, and links the menu's status lines (every connection)
    fn update(&mut self, status: LinkStatus, links: &[(u64, LinkStatus)], appearance: Appearance) {
        self.status = status;
        self.appearance = appearance;
        let rows = status_rows(links);
        self.replace_status_items(&rows, appearance);
        let lines: Vec<String> = rows
            .iter()
            .map(|(_, status)| status_text(*status))
            .collect();
        // It is a colored image, so show it as is rather than as a template image
        if let Err(error) = self.tray.set_icon(Some(icon(status, appearance))) {
            warn!("アイコンを変えられない: {error}");
        }
        let tooltip = format!("{}\n{}", config::APP_NAME, lines.join("\n"));
        if let Err(error) = self.tray.set_tooltip(Some(tooltip)) {
            warn!("ツールチップを変えられない: {error}");
        }
    }

    /// Matches the status lines to rows. Recreates the items only when the layout of the lines (connection number
    /// and whether connected) changes; otherwise replaces only the text and dot
    fn replace_status_items(&mut self, rows: &[(Option<u64>, LinkStatus)], appearance: Appearance) {
        let kinds: Vec<(Option<u64>, bool)> = rows
            .iter()
            .map(|(id, status)| (*id, matches!(status, LinkStatus::Connected(_))))
            .collect();
        if self.row_kinds != kinds {
            for item in self.status_items.drain(..) {
                if let Err(error) = self.menu.remove(item.menu_item()) {
                    warn!("メニューの項目を消せない: {error}");
                }
            }
            for (position, (id, status)) in rows.iter().enumerate() {
                let item = status_row(*id, *status);
                if let Err(error) = self.menu.insert(item.menu_item(), position) {
                    warn!("メニューの項目を足せない: {error}");
                }
                self.status_items.push(item);
            }
            self.row_kinds = kinds;
        }
        for (item, (_, status)) in self.status_items.iter().zip(rows) {
            item.set_status(*status, appearance);
        }
    }

    /// The number of the connection to close if the pressed item is a status line's "Disconnect"
    fn connection_to_disconnect(&self, id: &MenuId) -> Option<u64> {
        self.status_items
            .iter()
            .find_map(|item| item.disconnect_connection(id))
    }
}

/// Opens the QR code window. Brings it to the front if already open
fn show_qr(qr_window: &mut Option<QrWindow>, target: &EventLoopWindowTarget<UserEvent>) {
    if let Some(window) = qr_window {
        window.bring_to_front();
        return;
    }
    match QrWindow::open(target) {
        Ok(window) => *qr_window = Some(window),
        Err(error) => warn!("QR コードのウィンドウを開けない: {error}"),
    }
}

/// Closes the area window and finishes the export. area is the area on Export;
/// None on Cancel (closing with ×, including keys)
fn finish_export(
    app: &Arc<App>,
    state: &export_flow::ExportState,
    window: Option<ExportWindow>,
    area: Option<Rect>,
) {
    let Some(window) = window else {
        return;
    };
    let request = window.request();
    // Close the window before exporting (so it is gone before the save path dialog)
    drop(window);
    export_flow::finish(app.clone(), state, request, area);
}

/// Opens the canvas in this PC's default browser. The URL is the same as the QR code's (endpoint and token included),
/// so it connects just like opening from the QR code, and the approval dialog appears
fn open_canvas(app: &App) {
    let Some(url) = app.pairing.canvas_url() else {
        warn!("キャンバスの URL がまだ無い(サーバーの準備中)");
        return;
    };
    // Do not wait for the browser to launch (so the event loop is not blocked)
    if let Err(error) = open::that_detached(url.as_str()) {
        warn!("ブラウザを開けない: {error}");
    }
}

/// About in the menu. On macOS it is a plain item; pressing it brings the app to the front and then
/// shows the standard About panel (about::show). muda's PredefinedMenuItem::about is not used
/// (it does not bring the app to the front before showing the panel; for apps not in the Dock,
/// the panel may appear behind other windows)
#[cfg(target_os = "macos")]
type AboutItem = MenuItem;
/// On Windows, use muda's About (a message box)
#[cfg(target_os = "windows")]
type AboutItem = PredefinedMenuItem;

fn about_label() -> String {
    format!("About {}", config::APP_NAME)
}

#[cfg(target_os = "macos")]
fn about_item() -> AboutItem {
    MenuItem::new(about_label(), true, None)
}

#[cfg(target_os = "windows")]
fn about_item() -> AboutItem {
    PredefinedMenuItem::about(
        Some(&about_label()),
        Some(tray_icon::menu::AboutMetadata {
            name: Some(config::APP_NAME.into()),
            version: Some(crate::about::version().into()),
            license: Some(crate::about::APP_LICENSE.into()),
            copyright: Some(crate::about::APP_COPYRIGHT.into()),
            comments: Some(crate::about::THIRD_PARTY_NOTICE.into()),
            ..Default::default()
        }),
    )
}

/// The menu's "Export". The format is chosen in the submenu that opens to the side (shown the same way as "Disconnect" on a status line)
struct ExportMenu {
    row: Submenu,
    png: MenuItem,
    pdf: MenuItem,
}

impl ExportMenu {
    fn new() -> Self {
        let png = MenuItem::new("PNG", true, None);
        let pdf = MenuItem::new("PDF", true, None);
        let row = Submenu::with_items("Export", true, &[&png, &pdf])
            .expect("Export のサブメニューを作れない");
        Self { row, png, pdf }
    }

    /// If the pressed item is in this submenu, its format
    fn format(&self, id: &MenuId) -> Option<Format> {
        if *id == *self.png.id() {
            Some(Format::Png)
        } else if *id == *self.pdf.id() {
            Some(Format::Pdf)
        } else {
            None
        }
    }
}

/// Menu items (excluding status lines)
struct MenuItems {
    show_qr: MenuItem,
    open_canvas: MenuItem,
    export: ExportMenu,
    about: AboutItem,
    quit: MenuItem,
}

/// Creates the menu bar icon and menu. Status lines are inserted at the top by update
fn create_indicator(
    status: LinkStatus,
    links: &[(u64, LinkStatus)],
    appearance: Appearance,
    items: &MenuItems,
) -> Indicator {
    let menu = Menu::with_items(&[
        &PredefinedMenuItem::separator(),
        &items.show_qr,
        &items.open_canvas,
        &items.export.row,
        &items.about,
        &items.quit,
    ])
    .expect("メニューを作れない");
    // Menu shares its contents, so items can still be added and removed through this copy after handing it to the tray
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_icon(icon(status, appearance))
        .build()
        .expect("トレイのアイコンを作れない");
    let mut indicator = Indicator {
        tray,
        menu,
        status_items: Vec::new(),
        row_kinds: Vec::new(),
        status,
        appearance,
    };
    indicator.update(status, links, appearance);
    indicator
}

/// Runs the event loop on the main thread. Call start after registering the window-opening entry points, etc.
/// (so the first-run guide does not fail by trying to open before registration)
pub fn run(app: Arc<App>, start: impl FnOnce()) -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Do not show an icon in the Dock (mut is only used on macOS, so it is added here)
    #[cfg(target_os = "macos")]
    let event_loop = {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        let mut event_loop = event_loop;
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop
    };

    // Wake the event loop for menu actions and requests from other threads
    // (the tao usage described in the tray-icon documentation)
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    let proxy = event_loop.create_proxy();
    app.ui
        .set_qr_opener(move || proxy.send_event(UserEvent::ShowQr).is_ok());
    let proxy = event_loop.create_proxy();
    app.status.set_listener(move || {
        let _ = proxy.send_event(UserEvent::Status);
    });
    let proxy = event_loop.create_proxy();
    app.pairing.set_listener(move || {
        let _ = proxy.send_event(UserEvent::Pairing);
    });
    let proxy = event_loop.create_proxy();
    let export_state = export_flow::ExportState::default();
    start();

    let items = MenuItems {
        show_qr: MenuItem::new("Show QR Code", true, None),
        open_canvas: MenuItem::new("Open Canvas", true, None),
        export: ExportMenu::new(),
        about: about_item(),
        quit: MenuItem::new("Quit", true, None),
    };
    let mut indicator: Option<Indicator> = None;
    let mut qr_window: Option<QrWindow> = None;
    // The export area window (at most one at a time)
    let mut export_window: Option<ExportWindow> = None;
    // Invisible window used to learn the OS appearance (light / dark),
    // because tao reports the appearance only per window (Window::theme and WindowEvent::ThemeChanged)
    let mut theme_window: Option<Window> = None;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // Create it after the loop starts (per tray-icon: on macOS, from StartCause::Init on)
            Event::NewEvents(StartCause::Init) => {
                theme_window = WindowBuilder::new()
                    .with_visible(false)
                    .build(target)
                    .inspect_err(|error| warn!("外観を調べるウィンドウを作れない: {error}"))
                    .ok();
                let current = theme_window
                    .as_ref()
                    .map_or(Appearance::Light, |w| appearance(w.theme()));
                indicator = Some(create_indicator(
                    app.status.get(),
                    &app.status.all(),
                    current,
                    &items,
                ));
            }
            Event::UserEvent(UserEvent::ShowQr) => show_qr(&mut qr_window, target),
            Event::UserEvent(UserEvent::Status) => {
                let status = app.status.get();
                if let Some(indicator) = &mut indicator {
                    let appearance = indicator.appearance;
                    indicator.update(status, &app.status.all(), appearance);
                }
                // Once connected, the QR code is no longer needed
                if matches!(status, LinkStatus::Connected(_)) {
                    qr_window.take();
                }
            }
            Event::UserEvent(UserEvent::Pairing) => {
                // The QR contents changed: if the window is open, redraw it with the new contents
                if let Some(window) = &qr_window {
                    window.request_redraw();
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.show_qr.id() => {
                show_qr(&mut qr_window, target);
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.open_canvas.id() => {
                open_canvas(&app);
            }
            Event::UserEvent(UserEvent::Menu(event))
                if let Some(format) = items.export.format(&event.id) =>
            {
                // If an export is in progress (the area window is open), just bring it to the front
                if let Some(window) = &export_window {
                    window.bring_to_front();
                } else {
                    // The closure is created for each event, so clone the proxy each time
                    let proxy = proxy.clone();
                    export_flow::start(app.clone(), &export_state, format, move |request| {
                        let _ = proxy.send_event(UserEvent::ExportArea(request));
                    });
                }
            }
            Event::UserEvent(UserEvent::ExportArea(request)) => {
                let proxy = proxy.clone();
                // Pass a clone to open, so the request can be handed to finish if the window can't be opened
                match ExportWindow::open(target, &app, request.clone(), proxy) {
                    Ok(window) => export_window = Some(window),
                    Err(error) => {
                        warn!("範囲を選ぶウィンドウを開けない: {error}");
                        // Discard the export
                        export_flow::finish(app.clone(), &export_state, request, None);
                    }
                }
            }
            // The sheet of the area window was drawn on another thread.
            // Discard a result that arrived after the window closed
            Event::UserEvent(UserEvent::SheetRendered(window_id, job, pixels)) => {
                if let Some(window) = export_window.as_mut().filter(|w| w.id() == window_id) {
                    window.sheet_rendered(job, pixels);
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.about.id() => {
                crate::about::show();
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.quit.id() => {
                // Write the current state right away before quitting
                app.save_now();
                // Close the area window if it is open (discard the export)
                export_window.take();
                qr_window.take();
                indicator.take();
                *control_flow = ControlFlow::Exit;
            }
            // "Disconnect" was pressed on a connected status line
            Event::UserEvent(UserEvent::Menu(event)) => {
                if let Some(indicator) = &indicator
                    && let Some(connection) = indicator.connection_to_disconnect(&event.id)
                {
                    app.disconnects.request(connection);
                }
            }
            Event::RedrawRequested(id) => {
                if let Some(window) = qr_window.as_mut().filter(|w| w.id() == id) {
                    window.redraw(&app);
                } else if let Some(window) = export_window.as_mut().filter(|w| w.id() == id) {
                    window.redraw();
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::ThemeChanged(theme),
                ..
            } if theme_window.as_ref().is_some_and(|w| w.id() == window_id) => {
                if let Some(indicator) = &mut indicator {
                    let status = indicator.status;
                    indicator.update(status, &app.status.all(), appearance(theme));
                }
            }
            Event::WindowEvent {
                window_id, event, ..
            } if qr_window.as_ref().is_some_and(|w| w.id() == window_id) => match event {
                WindowEvent::CloseRequested => {
                    qr_window.take();
                }
                WindowEvent::Resized(_) => {
                    if let Some(window) = &qr_window {
                        window.request_redraw();
                    }
                }
                _ => {}
            },
            // Events of the area window (export). Events of the QR window are taken
            // by the arm above, so pass only the area window here
            Event::WindowEvent {
                window_id, event, ..
            } => {
                let Some(window) = export_window.as_mut().filter(|w| w.id() == window_id) else {
                    return;
                };
                match event {
                    // Closing the window (×) is Cancel
                    WindowEvent::CloseRequested => {
                        finish_export(&app, &export_state, export_window.take(), None);
                    }
                    _ => {
                        if let Some(outcome) = window.event(&event) {
                            let area = (outcome == Outcome::Export).then(|| window.area());
                            finish_export(&app, &export_state, export_window.take(), area);
                        }
                    }
                }
            }
            _ => {}
        }
    })
}
