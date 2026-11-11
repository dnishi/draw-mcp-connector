//! Menu bar (macOS) / notification area (Windows) icon, and the QR code window.
//! The app's only UI is the menu (connection status, "Show QR Code", "Open Canvas", "About", "Quit"), the QR code window,
//! and the connection approval dialog (approval_dialog.rs).
//! On macOS the event loop and icon must run on the main thread (per the tray-icon documentation).

use std::sync::Arc;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget};
use tao::window::{Theme, Window, WindowBuilder};
use tracing::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::app::App;
use crate::config;
use crate::icon::{self, Appearance, Badge};
use crate::link_status::LinkStatus;
use crate::qr_window::QrWindow;

/// Events delivered to the event loop
enum UserEvent {
    Menu(MenuEvent),
    /// Open the QR code window from another thread (MCP's connect_canvas, the first-run guide)
    ShowQr,
    /// The connection state with the canvas changed (read the current state from app.status)
    Status,
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

/// Status lines of the menu. One line per connection (in connection order). A single Not Connected line if there are none
fn status_lines(links: &[LinkStatus]) -> Vec<String> {
    if links.is_empty() {
        return vec![status_text(LinkStatus::Disconnected)];
    }
    links.iter().copied().map(status_text).collect()
}

/// A menu item for a status line. Pressing it does nothing, but it is enabled
/// (disabled items are shown in faint gray on macOS and are hard to read)
fn status_item(text: &str) -> MenuItem {
    MenuItem::new(text, true, None)
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
    status_items: Vec<MenuItem>,
    /// State to display, used for the icon's dot color (the state of the connection whose state changed last)
    status: LinkStatus,
    appearance: Appearance,
}

impl Indicator {
    /// status sets the icon's dot color, and links the menu's status lines (every connection)
    fn update(&mut self, status: LinkStatus, links: &[LinkStatus], appearance: Appearance) {
        self.status = status;
        self.appearance = appearance;
        let lines = status_lines(links);
        self.replace_status_items(&lines);
        // It is a colored image, so show it as is rather than as a template image
        if let Err(error) = self.tray.set_icon(Some(icon(status, appearance))) {
            warn!("アイコンを変えられない: {error}");
        }
        let tooltip = format!("{}\n{}", config::APP_NAME, lines.join("\n"));
        if let Err(error) = self.tray.set_tooltip(Some(tooltip)) {
            warn!("ツールチップを変えられない: {error}");
        }
    }

    /// Matches the status lines to lines. Recreates the items only when the number of lines changes
    fn replace_status_items(&mut self, lines: &[String]) {
        if self.status_items.len() != lines.len() {
            for item in self.status_items.drain(..) {
                if let Err(error) = self.menu.remove(&item) {
                    warn!("メニューの項目を消せない: {error}");
                }
            }
            for (position, line) in lines.iter().enumerate() {
                let item = status_item(line);
                if let Err(error) = self.menu.insert(&item, position) {
                    warn!("メニューの項目を足せない: {error}");
                }
                self.status_items.push(item);
            }
        }
        for (item, line) in self.status_items.iter().zip(lines) {
            item.set_text(line);
        }
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

/// About in the menu. On macOS, pressing it shows the standard dialog (about::show).
/// muda's standard About panel is not used (for apps not in the Dock it may appear behind other windows,
/// and this was not verified on real hardware; we use the method confirmed to come to the front for the approval dialog)
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

/// Menu items (excluding status lines)
struct MenuItems {
    show_qr: MenuItem,
    open_canvas: MenuItem,
    about: AboutItem,
    quit: MenuItem,
}

/// Creates the menu bar icon and menu. Status lines are inserted at the top by update
fn create_indicator(
    status: LinkStatus,
    links: &[LinkStatus],
    appearance: Appearance,
    items: &MenuItems,
) -> Indicator {
    let menu = Menu::with_items(&[
        &PredefinedMenuItem::separator(),
        &items.show_qr,
        &items.open_canvas,
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
    start();

    let items = MenuItems {
        show_qr: MenuItem::new("Show QR Code", true, None),
        open_canvas: MenuItem::new("Open Canvas", true, None),
        about: about_item(),
        quit: MenuItem::new("Quit", true, None),
    };
    let mut indicator: Option<Indicator> = None;
    let mut qr_window: Option<QrWindow> = None;
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
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.show_qr.id() => {
                show_qr(&mut qr_window, target);
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.open_canvas.id() => {
                open_canvas(&app);
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.about.id() => {
                crate::about::show();
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == *items.quit.id() => {
                qr_window.take();
                indicator.take();
                *control_flow = ControlFlow::Exit;
            }
            Event::RedrawRequested(id) => {
                if let Some(window) = qr_window.as_mut().filter(|w| w.id() == id) {
                    window.redraw(&app);
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
            _ => {}
        }
    })
}
