//! The flow of the "PNG" and "PDF" items under "Export" in the tray menu.
//! Dialogs block until there is an answer, so they are not shown on the main thread (the event loop);
//! they are shown on another thread, and when an area is needed, one is handed to the event loop
//! side through on_area (once the area is decided, finish is called)

use std::fs;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::app::App;
use crate::export::{self, Format, Target};
use crate::export_dialog;
use crate::protocol::PageSize;
use crate::render::Rect;

/// Whether an export is in progress (to ignore PNG or PDF while an export is in progress). Held by tray.rs
#[derive(Clone, Default)]
pub struct ExportState {
    in_progress: Arc<AtomicBool>,
}

impl ExportState {
    /// Makes it in progress. false if it already is
    fn begin(&self) -> bool {
        !self.in_progress.swap(true, Ordering::SeqCst)
    }

    /// Clears the in-progress state
    fn done(&self) {
        self.in_progress.store(false, Ordering::SeqCst);
    }
}

/// A guard for the in-progress state. Sets it back to false on Drop (Cancel, an error, a success,
/// and a panic of the thread all end in a Drop, so it always comes back). On a handoff it does
/// not set it back, and leaves that to the guard of the side it was handed to
struct Guard<'a> {
    state: &'a ExportState,
    /// Handed to finish (or the thread beyond it), so that side sets it back
    handoff: bool,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if !self.handoff {
            self.state.done();
        }
    }
}

/// The result before the area is decided. Returned to the event loop when an area has to be chosen
#[derive(Debug, Clone)]
pub struct AreaRequest {
    pub format: Format,
    pub target: Target,
    /// The ID of the current sheet at the moment the export started. Used for what the area window
    /// shows and for the Current Sheet export. Not used for All Sheets
    pub sheet: String,
    pub page: PageSize,
}

/// Starts the export in format (the format chosen in the menu). Does nothing if an export is already in progress.
/// For PDF, shows the target dialog on another thread, and when an area has to be chosen,
/// calls on_area (the side that decides the area calls finish)
pub fn start(
    app: Arc<App>,
    state: &ExportState,
    format: Format,
    on_area: impl FnOnce(AreaRequest) + Send + 'static,
) {
    if !state.begin() {
        // An export is in progress
        return;
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let mut guard = Guard {
            state: &state,
            handoff: false,
        };
        let target = match format {
            Format::Png => Target::CurrentSheet,
            Format::Pdf => match export_dialog::choose_target() {
                Some(target) => target,
                None => return, // Cancel
            },
        };
        let Some(page) = app.canvas.page() else {
            export_dialog::show_message(
                "Open the canvas once so the app can learn the sheet size.",
            );
            return;
        };
        // Remember the current sheet at the moment the export starts (even if the sheet is switched
        // while the area window is open, this sheet is exported)
        let sheet = app.canvas.snapshot().current;
        // In every ending, this thread does not set it back, and hands it on to the next side
        guard.handoff = true;
        let request = AreaRequest {
            format,
            target,
            sheet,
            page,
        };
        if target == Target::AllSheets {
            // No area is needed (always the whole sheet)
            finish(app, &state, request, Some(Rect::from(page)));
        } else {
            // The area is decided on the event loop side
            on_area(request);
        }
    });
}

/// Finishes the export with the area. If area is None (the area choice was Cancelled),
/// clears the in-progress state and ends
pub fn finish(app: Arc<App>, state: &ExportState, request: AreaRequest, area: Option<Rect>) {
    let Some(area) = area else {
        state.done();
        return;
    };
    // Keeps the area inside the sheet (the whole sheet if it does not fit)
    let area = export::clamp_to_page(area, request.page).unwrap_or(Rect::from(request.page));
    let state = state.clone();
    std::thread::spawn(move || {
        // Sets the in-progress state back when the export ends (or is cut off partway)
        let _guard = Guard {
            state: &state,
            handoff: false,
        };
        run_export(&app, &request, area);
    });
}

/// Chooses the save path, makes the PNG / PDF, and writes it to the file
fn run_export(app: &App, request: &AreaRequest, area: Rect) {
    let stamp = export_dialog::local_stamp();
    let sheet = (request.target == Target::CurrentSheet)
        .then(|| app.canvas.snapshot().sheet_number(&request.sheet))
        .flatten();
    let name = export::default_file_name(stamp.as_deref(), sheet, request.format);
    let Some(path) = export_dialog::choose_save_path(&name, request.format) else {
        return; // Cancel of the save path
    };
    let path = export::with_extension(&path, request.format);
    let bytes = match request.format {
        Format::Png => export::sheet_png_by_id(&app.canvas, &request.sheet, area),
        Format::Pdf => export::pdf(
            &app.canvas,
            request.target,
            &request.sheet,
            area,
            request.page,
        ),
    };
    match bytes {
        Ok(bytes) => {
            if let Err(error) = fs::write(&path, bytes) {
                export_dialog::show_message(&format!("Couldn't export: {error}"));
            }
        }
        Err(reason) => export_dialog::show_message(&format!("Couldn't export: {reason}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_guard_clears_the_flag_when_it_is_dropped() {
        let state = ExportState::default();
        assert!(state.begin());
        {
            let _guard = Guard {
                state: &state,
                handoff: false,
            };
            assert!(!state.begin(), "途中");
        }
        assert!(state.begin(), "Drop で戻る");
    }

    #[test]
    fn a_handed_off_guard_leaves_the_flag_to_the_other_side() {
        let state = ExportState::default();
        assert!(state.begin());
        {
            let _guard = Guard {
                state: &state,
                handoff: true,
            };
        }
        assert!(!state.begin(), "handoff のときは戻さない");
        state.done();
        assert!(state.begin(), "戻すと開く");
    }

    #[test]
    fn start_does_nothing_while_an_export_is_running() {
        let state = ExportState::default();
        state.begin();
        let app = Arc::new(App::new());
        start(app, &state, Format::Png, |_| {
            panic!("途中なら何も呼び出されない")
        });
        // If a thread had been spawned while in progress, the in-progress state would come back before this return
        std::thread::sleep(Duration::from_millis(100));
        assert!(!state.begin(), "まだ途中");
    }
}
