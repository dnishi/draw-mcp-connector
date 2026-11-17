//! Canvas state held by the bridge. Changes from both the canvas page and the AI go through here.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::lock::lock;
use crate::protocol::{
    CanvasOp, INITIAL_SHEET_ID, ImagePlacement, MAX_SHEETS, PageSize, Sheet, Stroke, Viewport,
};
use crate::render::StoredImage;

/// Number of changes remembered to return diffs. Diffs from older revisions are returned as the whole canvas
const MAX_LOG_LEN: usize = 10_000;

/// Number of image byte arrays to keep. Used when undo/redo brings back an old ID, and
/// for handing them to pages that connect later. The oldest are dropped first
const MAX_IMAGE_ENTRIES: usize = 8;

/// Who made the change. Used to avoid sending a change back to its sender
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A connection to a canvas page (numbered per connection)
    Connection(u64),
    Ai,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub origin: Origin,
    pub op: CanvasOp,
}

/// How one connected canvas page sees the canvas. Several devices may connect at once;
/// strokes are shared by all, but the paper size (screen size) and visible area differ per device
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CanvasView {
    /// Connection number (assigned per connection)
    pub id: u64,
    /// IP address of the device that has the page open
    pub ip: IpAddr,
    /// Paper size. None if the page has not sent it
    pub page: Option<PageSize>,
    /// Area currently visible on the page
    pub viewport: Viewport,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    /// Revision number, incremented by 1 on every change
    pub revision: u64,
    /// Connected canvas pages (in connection order). Empty if none are connected
    pub views: Vec<CanvasView>,
    /// Sheets (in stacking order)
    pub sheets: Vec<Sheet>,
    /// ID of the current sheet. Always present in `sheets`
    pub current: String,
}

impl Snapshot {
    /// The current sheet. It always exists by the state's rules
    pub fn current_sheet(&self) -> &Sheet {
        self.sheets
            .iter()
            .find(|sheet| sheet.id == self.current)
            .expect("今の紙は必ず存在する")
    }

    /// Number of the sheet (counted from 1)
    pub fn sheet_number(&self, id: &str) -> Option<usize> {
        self.sheets
            .iter()
            .position(|sheet| sheet.id == id)
            .map(|position| position + 1)
    }
}

/// Images looked up by ID (the bytes and their decoded result). Kept in arrival order; the oldest are dropped when there are too many.
/// When the contents for the same ID arrive again the whole entry is replaced, so no decoded result of the old contents remains
#[derive(Default)]
struct ImageData {
    entries: VecDeque<(String, Arc<StoredImage>)>,
}

impl ImageData {
    /// Never drops the IDs of placed images. Over the limit, drops the oldest of the images
    /// that are not placed, and drops nothing if every image is placed
    fn put(&mut self, id: &str, image: Arc<StoredImage>, placed: &HashSet<String>) {
        if let Some(entry) = self.entries.iter_mut().find(|(stored, _)| stored == id) {
            entry.1 = image;
        } else {
            self.entries.push_back((id.to_string(), image));
        }
        while self.entries.len() > MAX_IMAGE_ENTRIES {
            let Some(index) = self
                .entries
                .iter()
                .position(|(stored, _)| !placed.contains(stored))
            else {
                break;
            };
            self.entries.remove(index);
        }
    }

    fn get(&self, id: &str) -> Option<Arc<StoredImage>> {
        self.entries
            .iter()
            .find(|(stored, _)| stored == id)
            .map(|(_, image)| Arc::clone(image))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChangesSince {
    /// Changes that, applied in order from the given revision, produce the current state
    Ops { revision: u64, ops: Vec<CanvasOp> },
    /// The given revision is too old to produce a diff. Returns the whole canvas
    Snapshot(Snapshot),
}

struct State {
    /// Sheets (in stacking order)
    sheets: Vec<Sheet>,
    /// ID of the current sheet. Always present in `sheets`
    current: String,
    /// Image bytes (looked up by placement ID)
    image_data: ImageData,
    revision: u64,
    /// In order of connection number
    views: Vec<CanvasView>,
    /// (revision after the change, change)
    log: VecDeque<(u64, CanvasOp)>,
}

/// Makes an empty sheet with ID `id`
fn empty_sheet(id: &str) -> Sheet {
    Sheet {
        id: id.into(),
        strokes: Vec::new(),
        image: None,
    }
}

impl Default for State {
    /// Starts with one empty `INITIAL_SHEET_ID` sheet
    fn default() -> Self {
        Self {
            sheets: vec![empty_sheet(INITIAL_SHEET_ID)],
            current: INITIAL_SHEET_ID.into(),
            image_data: ImageData::default(),
            revision: 0,
            views: Vec::new(),
            log: VecDeque::new(),
        }
    }
}

impl State {
    fn sheet(&self, id: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|sheet| sheet.id == id)
    }

    fn sheet_mut(&mut self, id: &str) -> Option<&mut Sheet> {
        self.sheets.iter_mut().find(|sheet| sheet.id == id)
    }

    /// IDs of the images placed on all sheets
    fn placed_image_ids(&self) -> HashSet<String> {
        self.sheets
            .iter()
            .filter_map(|sheet| sheet.image.as_ref().map(|image| image.id.clone()))
            .collect()
    }

    /// Applies a change and returns the change that actually happened (None if nothing changed)
    fn apply(&mut self, op: CanvasOp) -> Option<CanvasOp> {
        match op {
            CanvasOp::Insert {
                sheet,
                stroke,
                index,
            } => {
                let target = self.sheet_mut(&sheet)?;
                if target.strokes.iter().any(|s| s.id == stroke.id) {
                    return None;
                }
                let index = index.min(target.strokes.len());
                target.strokes.insert(index, stroke.clone());
                Some(CanvasOp::Insert {
                    sheet,
                    stroke,
                    index,
                })
            }
            CanvasOp::Remove { sheet, ids } => {
                let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
                let target = self.sheet_mut(&sheet)?;
                let mut removed = Vec::new();
                target.strokes.retain(|stroke| {
                    let hit = requested.contains(stroke.id.as_str());
                    if hit {
                        removed.push(stroke.id.clone());
                    }
                    !hit
                });
                (!removed.is_empty()).then_some(CanvasOp::Remove {
                    sheet,
                    ids: removed,
                })
            }
            CanvasOp::Replace { sheet, stroke } => {
                let target = self.sheet_mut(&sheet)?;
                let index = target.strokes.iter().position(|s| s.id == stroke.id)?;
                target.strokes[index] = stroke.clone();
                Some(CanvasOp::Replace { sheet, stroke })
            }
            CanvasOp::SetImage { sheet, image } => {
                let target = self.sheet_mut(&sheet)?;
                if target.image.as_ref() == Some(&image) {
                    return None;
                }
                target.image = Some(image.clone());
                Some(CanvasOp::SetImage { sheet, image })
            }
            CanvasOp::RemoveImage { sheet } => self
                .sheet_mut(&sheet)?
                .image
                .take()
                .map(|_| CanvasOp::RemoveImage { sheet }),
            CanvasOp::AddSheet { id, index } => {
                if self.sheets.len() >= MAX_SHEETS || self.sheets.iter().any(|sheet| sheet.id == id)
                {
                    return None;
                }
                let index = index.min(self.sheets.len());
                self.sheets.insert(index, empty_sheet(&id));
                Some(CanvasOp::AddSheet { id, index })
            }
            CanvasOp::RemoveSheet { id } => {
                let position = self.sheets.iter().position(|sheet| sheet.id == id)?;
                if self.sheets.len() <= 1 {
                    return None;
                }
                self.sheets.remove(position);
                if self.current == id {
                    // When the current sheet is removed, the sheet at that position (or the new last sheet) becomes current
                    self.current = self.sheets[position.min(self.sheets.len() - 1)].id.clone();
                }
                Some(CanvasOp::RemoveSheet { id })
            }
            CanvasOp::SelectSheet { id } => {
                if self.current == id || self.sheet(&id).is_none() {
                    return None;
                }
                self.current = id.clone();
                Some(CanvasOp::SelectSheet { id })
            }
            CanvasOp::ResetSheets => {
                // Ignored if there is already just one empty INITIAL_SHEET_ID sheet (with one sheet, it is the current one)
                if self.sheets.len() == 1
                    && self.sheets[0].id == INITIAL_SHEET_ID
                    && self.sheets[0].strokes.is_empty()
                    && self.sheets[0].image.is_none()
                {
                    return None;
                }
                self.sheets = vec![empty_sheet(INITIAL_SHEET_ID)];
                self.current = INITIAL_SHEET_ID.into();
                Some(CanvasOp::ResetSheets)
            }
        }
    }

    fn record(&mut self, op: &CanvasOp) {
        self.revision += 1;
        self.log.push_back((self.revision, op.clone()));
        if self.log.len() > MAX_LOG_LEN {
            self.log.pop_front();
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            views: self.views.clone(),
            sheets: self.sheets.clone(),
            current: self.current.clone(),
        }
    }
}

pub struct Canvas {
    state: Mutex<State>,
    changes: broadcast::Sender<Change>,
    /// Tells connections that an image's contents have arrived (same mechanism as stroke changes)
    images_ready: broadcast::Sender<String>,
}

impl Canvas {
    pub fn new() -> Self {
        Self::with_state(State::default())
    }

    /// Makes a Canvas from the saved state. Views and the change log start empty
    pub fn restored(saved: crate::store::Saved) -> Self {
        let mut state = State {
            sheets: saved.sheets,
            current: saved.current,
            image_data: ImageData::default(),
            revision: saved.revision,
            views: Vec::new(),
            log: VecDeque::new(),
        };
        // Every saved image is placed, so none is dropped
        let placed = state.placed_image_ids();
        for (id, bytes) in saved.images {
            state
                .image_data
                .put(&id, Arc::new(StoredImage::new(bytes)), &placed);
        }
        Self::with_state(state)
    }

    /// Shares how the change channels are made
    fn with_state(state: State) -> Self {
        Self {
            state: Mutex::new(state),
            changes: broadcast::channel(1024).0,
            // A short queue that just remembers what arrived is enough. Missed notices are recovered by recounting
            images_ready: broadcast::channel(64).0,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Applies a change and notifies the other parties. Returns the change that actually happened
    pub fn apply(&self, origin: Origin, op: CanvasOp) -> Option<CanvasOp> {
        let mut state = self.state();
        let applied = state.apply(op)?;
        state.record(&applied);
        // A send failure with no receivers is fine. Send while locked so the send order matches the revision order
        let _ = self.changes.send(Change {
            origin,
            op: applied.clone(),
        });
        Some(applied)
    }

    /// Remembers the visible area and paper size of a connected canvas page (does not bump the revision)
    pub fn set_view(&self, view: CanvasView) {
        let mut state = self.state();
        match state.views.binary_search_by_key(&view.id, |v| v.id) {
            Ok(index) => state.views[index] = view,
            Err(index) => state.views.insert(index, view),
        }
    }

    /// Forgets how a closed connection saw the canvas
    pub fn remove_view(&self, id: u64) {
        self.state().views.retain(|view| view.id != id);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state().snapshot()
    }

    /// Looks up a stroke by ID (searching all sheets). Returns the sheet ID and the stroke
    pub fn find_stroke(&self, id: &str) -> Option<(String, Stroke)> {
        self.state().sheets.iter().find_map(|sheet| {
            sheet
                .strokes
                .iter()
                .find(|stroke| stroke.id == id)
                .map(|stroke| (sheet.id.clone(), stroke.clone()))
        })
    }

    pub fn changes_since(&self, revision: u64) -> ChangesSince {
        let state = self.state();
        let oldest_known = state.log.front().map_or(state.revision, |(rev, _)| rev - 1);
        if revision < oldest_known || revision > state.revision {
            return ChangesSince::Snapshot(state.snapshot());
        }
        let ops = state
            .log
            .iter()
            .filter(|(rev, _)| *rev > revision)
            .map(|(_, op)| op.clone())
            .collect();
        ChangesSince::Ops {
            revision: state.revision,
            ops,
        }
    }

    /// Gets a full copy and a receiver for later changes at the same time (no change in between is missed)
    pub fn subscribe(&self) -> (Snapshot, broadcast::Receiver<Change>) {
        let state = self.state();
        (state.snapshot(), self.changes.subscribe())
    }

    /// Stores an image's bytes (they may arrive before the placement). The same ID is replaced.
    /// Announces the arrived ID so that connections waiting for the contents can act on it
    pub fn put_image_data(&self, id: &str, bytes: Vec<u8>) {
        let mut state = self.state();
        let placed = state.placed_image_ids();
        state
            .image_data
            .put(id, Arc::new(StoredImage::new(bytes)), &placed);
        // A failed send with no receivers is fine (same as stroke changes)
        let _ = self.images_ready.send(id.to_string());
    }

    /// Looks up an image by placement ID. Decoding the PNG happens after taking it out here (outside the lock)
    pub fn image_data(&self, id: &str) -> Option<Arc<StoredImage>> {
        self.state().image_data.get(id)
    }

    /// Placement of the image with that ID, on whichever sheet it is placed
    pub fn placed_image(&self, id: &str) -> Option<ImagePlacement> {
        self.state()
            .sheets
            .iter()
            .find_map(|sheet| sheet.image.as_ref().filter(|image| image.id == id).cloned())
    }

    /// IDs of the images placed on all sheets
    pub fn placed_image_ids(&self) -> HashSet<String> {
        self.state().placed_image_ids()
    }

    /// Receiver for notices that an image's contents arrived (used to send images requested before their contents arrived)
    pub fn subscribe_images(&self) -> broadcast::Receiver<String> {
        self.images_ready.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Author, StrokePoint};

    fn stroke(id: &str) -> Stroke {
        Stroke {
            id: id.into(),
            author: Author::Human,
            color: crate::color::BLACK.into(),
            points: vec![StrokePoint {
                x: 0.0,
                y: 0.0,
                pressure: 0.5,
            }],
        }
    }

    /// A change to the current sheet (= `INITIAL_SHEET_ID`)
    fn insert(id: &str, index: usize) -> CanvasOp {
        CanvasOp::Insert {
            sheet: INITIAL_SHEET_ID.into(),
            stroke: stroke(id),
            index,
        }
    }

    fn remove(ids: Vec<String>) -> CanvasOp {
        CanvasOp::Remove {
            sheet: INITIAL_SHEET_ID.into(),
            ids,
        }
    }

    fn ids(canvas: &Canvas) -> Vec<String> {
        canvas
            .snapshot()
            .current_sheet()
            .strokes
            .iter()
            .map(|s| s.id.clone())
            .collect()
    }

    /// IDs of the sheets
    fn sheet_ids(canvas: &Canvas) -> Vec<String> {
        canvas
            .snapshot()
            .sheets
            .iter()
            .map(|s| s.id.clone())
            .collect()
    }

    #[test]
    fn insert_clamps_index_and_ignores_duplicate_ids() {
        let canvas = Canvas::new();
        canvas.apply(Origin::Ai, insert("a", 0));
        let applied = canvas.apply(Origin::Ai, insert("b", 99));
        assert_eq!(applied, Some(insert("b", 1)));
        canvas.apply(Origin::Ai, insert("c", 1));
        assert_eq!(ids(&canvas), ["a", "c", "b"]);
        assert_eq!(canvas.apply(Origin::Ai, insert("a", 0)), None);
    }

    #[test]
    fn remove_reports_only_existing_ids() {
        let canvas = Canvas::new();
        canvas.apply(Origin::Ai, insert("a", 0));
        let applied = canvas.apply(Origin::Ai, remove(vec!["x".into(), "a".into()]));
        assert_eq!(applied, Some(remove(vec!["a".into()])));
        assert_eq!(canvas.apply(Origin::Ai, remove(vec!["x".into()])), None);
        assert_eq!(canvas.snapshot().revision, 2);
    }

    #[test]
    fn add_sheet_inserts_ignores_duplicates_and_the_limit() {
        let canvas = Canvas::new();
        let add = |id: &str, index: usize| CanvasOp::AddSheet {
            id: id.into(),
            index,
        };

        // Inserts at the position (at the end if out of range). The current sheet does not change
        assert_eq!(canvas.apply(Origin::Ai, add("b", 0)), Some(add("b", 0)));
        assert_eq!(canvas.apply(Origin::Ai, add("c", 99)), Some(add("c", 2)));
        assert_eq!(sheet_ids(&canvas), ["b", INITIAL_SHEET_ID, "c"]);
        assert_eq!(canvas.snapshot().current, INITIAL_SHEET_ID);

        // The same ID is ignored
        assert_eq!(canvas.apply(Origin::Ai, add("b", 0)), None);

        // Adds up to the limit and ignores any more
        for i in 3..MAX_SHEETS {
            assert!(canvas.apply(Origin::Ai, add(&format!("s{i}"), 0)).is_some());
        }
        assert_eq!(canvas.snapshot().sheets.len(), MAX_SHEETS);
        assert_eq!(canvas.apply(Origin::Ai, add("one-too-many", 0)), None);
    }

    #[test]
    fn remove_sheet_makes_the_next_sheet_current() {
        let canvas = Canvas::new();
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "a".into(),
                index: 0,
            },
        );
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 2,
            },
        );
        assert_eq!(sheet_ids(&canvas), ["a", INITIAL_SHEET_ID, "b"]);

        // Removing the current sheet makes the sheet at that position current
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::RemoveSheet {
                    id: INITIAL_SHEET_ID.into()
                }
            ),
            Some(CanvasOp::RemoveSheet {
                id: INITIAL_SHEET_ID.into()
            })
        );
        assert_eq!(canvas.snapshot().current, "b");

        // Removing the last sheet makes the new last sheet current
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::RemoveSheet { id: "b".into() }),
            Some(CanvasOp::RemoveSheet { id: "b".into() })
        );
        assert_eq!(canvas.snapshot().current, "a");

        // The last remaining sheet is ignored, and so is a missing sheet
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::RemoveSheet { id: "a".into() }),
            None
        );
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::RemoveSheet { id: "none".into() }),
            None
        );
        assert_eq!(sheet_ids(&canvas), ["a"]);
    }

    #[test]
    fn select_sheet_ignores_missing_and_current() {
        let canvas = Canvas::new();
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::SelectSheet { id: "b".into() }),
            Some(CanvasOp::SelectSheet { id: "b".into() })
        );
        assert_eq!(canvas.snapshot().current, "b");
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::SelectSheet { id: "b".into() }),
            None,
            "同じ紙は無視"
        );
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::SelectSheet { id: "none".into() }),
            None,
            "無い紙は無視"
        );
        assert_eq!(canvas.snapshot().current, "b");
    }

    #[test]
    fn reset_sheets_leaves_one_blank_sheet() {
        let canvas = Canvas::new();
        canvas.apply(Origin::Ai, insert("a", 0));
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::ResetSheets),
            Some(CanvasOp::ResetSheets)
        );
        let snapshot = canvas.snapshot();
        assert_eq!(snapshot.sheets.len(), 1);
        assert_eq!(snapshot.sheets[0].id, INITIAL_SHEET_ID);
        assert!(snapshot.sheets[0].strokes.is_empty());
        assert_eq!(snapshot.current, INITIAL_SHEET_ID);

        // Ignored when already in that state
        assert_eq!(canvas.apply(Origin::Ai, CanvasOp::ResetSheets), None);
    }

    #[test]
    fn stroke_ops_on_a_missing_sheet_are_ignored() {
        let canvas = Canvas::new();
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::Insert {
                    sheet: "none".into(),
                    stroke: stroke("a"),
                    index: 0
                }
            ),
            None
        );
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::Remove {
                    sheet: "none".into(),
                    ids: vec!["a".into()]
                }
            ),
            None
        );
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::Replace {
                    sheet: "none".into(),
                    stroke: stroke("a")
                }
            ),
            None
        );
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::SetImage {
                    sheet: "none".into(),
                    image: placement("img")
                }
            ),
            None
        );
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::RemoveImage {
                    sheet: "none".into()
                }
            ),
            None
        );
        assert!(canvas.snapshot().current_sheet().strokes.is_empty());
    }

    #[test]
    fn sheets_do_not_share_strokes() {
        let canvas = Canvas::new();
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        // Strokes with the same ID on different sheets (duplicates are checked per sheet)
        canvas.apply(
            Origin::Ai,
            CanvasOp::Insert {
                sheet: INITIAL_SHEET_ID.into(),
                stroke: stroke("a"),
                index: 0,
            },
        );
        canvas.apply(
            Origin::Ai,
            CanvasOp::Insert {
                sheet: "b".into(),
                stroke: stroke("a"),
                index: 0,
            },
        );
        let snapshot = canvas.snapshot();
        assert_eq!(snapshot.sheets[0].strokes.len(), 1);
        assert_eq!(snapshot.sheets[1].strokes.len(), 1);

        // Removing from one sheet does not affect the other
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::Remove {
                    sheet: "b".into(),
                    ids: vec!["a".into()]
                }
            ),
            Some(CanvasOp::Remove {
                sheet: "b".into(),
                ids: vec!["a".into()]
            })
        );
        let snapshot = canvas.snapshot();
        assert_eq!(snapshot.sheets[0].strokes.len(), 1);
        assert!(snapshot.sheets[1].strokes.is_empty());
    }

    #[test]
    fn changes_since_returns_ops_or_snapshot() {
        let canvas = Canvas::new();
        canvas.apply(Origin::Ai, insert("a", 0));
        canvas.apply(Origin::Ai, insert("b", 1));

        let ChangesSince::Ops { revision, ops } = canvas.changes_since(1) else {
            panic!("差分を返すはず");
        };
        assert_eq!(revision, 2);
        assert_eq!(ops, [insert("b", 1)]);

        assert!(matches!(canvas.changes_since(5), ChangesSince::Snapshot(_)));
    }

    #[test]
    fn views_are_kept_per_connection_in_connection_order() {
        let canvas = Canvas::new();
        let view = |id: u64, width: f64| CanvasView {
            id,
            ip: IpAddr::from([192, 168, 0, id as u8]),
            page: Some(PageSize {
                width,
                height: 600.0,
            }),
            viewport: Viewport {
                x: 0.0,
                y: 0.0,
                width,
                height: 600.0,
            },
        };
        canvas.set_view(view(2, 800.0));
        canvas.set_view(view(1, 1200.0));
        // When it comes from the same connection, replace it (zoom, pan, screen size change)
        canvas.set_view(view(2, 400.0));
        assert_eq!(canvas.snapshot().views, [view(1, 1200.0), view(2, 400.0)]);

        canvas.remove_view(1);
        assert_eq!(canvas.snapshot().views, [view(2, 400.0)]);
    }

    #[test]
    fn subscribers_receive_origin() {
        let canvas = Canvas::new();
        let (_, mut changes) = canvas.subscribe();
        canvas.apply(Origin::Connection(7), insert("a", 0));
        let change = changes.try_recv().unwrap();
        assert_eq!(change.origin, Origin::Connection(7));
    }

    fn placement(id: &str) -> ImagePlacement {
        ImagePlacement {
            id: id.into(),
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        }
    }

    #[test]
    fn image_ops_keep_a_single_image_per_sheet() {
        let canvas = Canvas::new();
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::RemoveImage {
                    sheet: INITIAL_SHEET_ID.into()
                }
            ),
            None,
            "画像が無ければ無視"
        );
        let set = CanvasOp::SetImage {
            sheet: INITIAL_SHEET_ID.into(),
            image: placement("a"),
        };
        assert_eq!(canvas.apply(Origin::Ai, set.clone()), Some(set.clone()));
        assert_eq!(canvas.apply(Origin::Ai, set), None, "同じ配置なら変更なし");
        let moved = CanvasOp::SetImage {
            sheet: INITIAL_SHEET_ID.into(),
            image: ImagePlacement {
                x: 9.0,
                ..placement("a")
            },
        };
        assert_eq!(canvas.apply(Origin::Ai, moved.clone()), Some(moved));
        // Only one image, so placing another ID replaces it
        let swapped = CanvasOp::SetImage {
            sheet: INITIAL_SHEET_ID.into(),
            image: placement("b"),
        };
        assert_eq!(canvas.apply(Origin::Ai, swapped.clone()), Some(swapped));
        assert_eq!(
            canvas.snapshot().current_sheet().image,
            Some(placement("b"))
        );
        assert_eq!(
            canvas.apply(
                Origin::Ai,
                CanvasOp::RemoveImage {
                    sheet: INITIAL_SHEET_ID.into()
                }
            ),
            Some(CanvasOp::RemoveImage {
                sheet: INITIAL_SHEET_ID.into()
            })
        );
        assert_eq!(canvas.snapshot().current_sheet().image, None);
    }

    #[test]
    fn image_data_is_kept_by_id_and_evicted_from_the_oldest() {
        let canvas = Canvas::new();
        canvas.put_image_data("a", vec![1]);
        canvas.put_image_data("a", vec![2]);
        assert_eq!(canvas.image_data("a").unwrap().bytes(), [2]);
        for i in 0..MAX_IMAGE_ENTRIES {
            canvas.put_image_data(&format!("x{i}"), vec![i as u8]);
        }
        assert!(canvas.image_data("a").is_none(), "古い順に捨てる");
        assert!(canvas.image_data("x0").is_some());
    }

    #[test]
    fn placed_image_data_is_not_evicted() {
        let canvas = Canvas::new();
        canvas.apply(
            Origin::Ai,
            CanvasOp::SetImage {
                sheet: INITIAL_SHEET_ID.into(),
                image: placement("a"),
            },
        );
        canvas.put_image_data("a", vec![1]);
        for i in 0..MAX_IMAGE_ENTRIES {
            canvas.put_image_data(&format!("x{i}"), vec![i as u8]);
        }
        assert!(
            canvas.image_data("a").is_some(),
            "置かれている画像は捨てない"
        );
        assert!(
            canvas.image_data("x0").is_none(),
            "置かれていないものは最も古い順に捨てる"
        );
    }

    #[test]
    fn all_placed_images_are_kept_beyond_the_limit() {
        let canvas = Canvas::new();
        // One image per sheet, all placed, kept beyond the limit
        for i in 0..=MAX_IMAGE_ENTRIES {
            let sheet = if i == 0 {
                INITIAL_SHEET_ID.to_string()
            } else {
                format!("sh{i}")
            };
            if i > 0 {
                assert!(
                    canvas
                        .apply(
                            Origin::Ai,
                            CanvasOp::AddSheet {
                                id: sheet.clone(),
                                index: 0
                            }
                        )
                        .is_some()
                );
            }
            let id = format!("img{i}");
            assert!(
                canvas
                    .apply(
                        Origin::Ai,
                        CanvasOp::SetImage {
                            sheet: sheet.clone(),
                            image: placement(&id)
                        }
                    )
                    .is_some()
            );
            canvas.put_image_data(&id, vec![i as u8]);
        }
        for i in 0..=MAX_IMAGE_ENTRIES {
            assert!(
                canvas.image_data(&format!("img{i}")).is_some(),
                "置かれているものは捨てない"
            );
        }
    }

    #[test]
    fn put_image_data_notifies_subscribers() {
        let canvas = Canvas::new();
        let mut ready = canvas.subscribe_images();
        canvas.put_image_data("a", vec![1]);
        assert_eq!(ready.try_recv().unwrap(), "a");
    }

    #[test]
    fn restored_canvas_keeps_sheets_current_revision_and_images() {
        use crate::store::Saved;

        let saved = Saved {
            revision: 2,
            current: "b".into(),
            sheets: vec![
                Sheet {
                    id: "a".into(),
                    strokes: vec![stroke("s")],
                    image: None,
                },
                Sheet {
                    id: "b".into(),
                    strokes: Vec::new(),
                    image: Some(placement("img")),
                },
            ],
            images: vec![("img".into(), vec![7])],
        };
        let canvas = Canvas::restored(saved);
        let snapshot = canvas.snapshot();
        assert_eq!(snapshot.sheets.len(), 2);
        assert_eq!(snapshot.sheets[1].image, Some(placement("img")));
        assert_eq!(snapshot.current, "b");
        assert_eq!(snapshot.revision, 2);
        assert!(snapshot.views.is_empty());
        assert_eq!(canvas.image_data("img").unwrap().bytes(), [7]);

        // The change log is empty, so a revision before the saved one returns the whole canvas and the same revision returns an empty diff
        assert!(matches!(canvas.changes_since(1), ChangesSince::Snapshot(_)));
        let ChangesSince::Ops { revision, ops } = canvas.changes_since(2) else {
            panic!("同じ版なら差分を返すはず");
        };
        assert_eq!(revision, 2);
        assert!(ops.is_empty());
    }
}
