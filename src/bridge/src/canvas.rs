//! Canvas state held by the bridge. Changes from both the canvas page and the AI go through here.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::lock::lock;
use crate::protocol::{CanvasOp, ImagePlacement, PageSize, Stroke, Viewport};
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
    /// Stacking order (later is on top)
    pub strokes: Vec<Stroke>,
    /// The image placed under the strokes (one at most for now)
    pub image: Option<ImagePlacement>,
}

/// Images looked up by ID (the bytes and their decoded result). Kept in arrival order; the oldest are dropped when there are too many.
/// When the contents for the same ID arrive again the whole entry is replaced, so no decoded result of the old contents remains
#[derive(Default)]
struct ImageData {
    entries: VecDeque<(String, Arc<StoredImage>)>,
}

impl ImageData {
    fn put(&mut self, id: &str, image: Arc<StoredImage>) {
        if let Some(entry) = self.entries.iter_mut().find(|(stored, _)| stored == id) {
            entry.1 = image;
        } else {
            self.entries.push_back((id.to_string(), image));
        }
        while self.entries.len() > MAX_IMAGE_ENTRIES {
            self.entries.pop_front();
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

#[derive(Default)]
struct State {
    strokes: Vec<Stroke>,
    /// The image placed under the strokes (one at most for now)
    image: Option<ImagePlacement>,
    /// Image byte arrays (looked up by the placement ID)
    image_data: ImageData,
    revision: u64,
    /// In order of connection number
    views: Vec<CanvasView>,
    /// (revision after the change, change)
    log: VecDeque<(u64, CanvasOp)>,
}

impl State {
    fn index_of(&self, id: &str) -> Option<usize> {
        self.strokes.iter().position(|stroke| stroke.id == id)
    }

    /// Applies a change and returns the change that actually happened (None if nothing changed)
    fn apply(&mut self, op: CanvasOp) -> Option<CanvasOp> {
        match op {
            CanvasOp::Insert { stroke, index } => {
                if self.index_of(&stroke.id).is_some() {
                    return None;
                }
                let index = index.min(self.strokes.len());
                self.strokes.insert(index, stroke.clone());
                Some(CanvasOp::Insert { stroke, index })
            }
            CanvasOp::Remove { ids } => {
                let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
                let mut removed = Vec::new();
                self.strokes.retain(|stroke| {
                    let hit = requested.contains(stroke.id.as_str());
                    if hit {
                        removed.push(stroke.id.clone());
                    }
                    !hit
                });
                (!removed.is_empty()).then_some(CanvasOp::Remove { ids: removed })
            }
            CanvasOp::Replace { stroke } => {
                let index = self.index_of(&stroke.id)?;
                self.strokes[index] = stroke.clone();
                Some(CanvasOp::Replace { stroke })
            }
            CanvasOp::SetImage { image } => {
                if self.image.as_ref() == Some(&image) {
                    return None;
                }
                self.image = Some(image.clone());
                Some(CanvasOp::SetImage { image })
            }
            CanvasOp::RemoveImage => self.image.take().map(|_| CanvasOp::RemoveImage),
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
            strokes: self.strokes.clone(),
            image: self.image.clone(),
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
        Self {
            state: Mutex::default(),
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

    pub fn stroke(&self, id: &str) -> Option<Stroke> {
        let state = self.state();
        state.index_of(id).map(|index| state.strokes[index].clone())
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
        self.state()
            .image_data
            .put(id, Arc::new(StoredImage::new(bytes)));
        // Failing to send when there is no receiver is fine (same as stroke changes)
        let _ = self.images_ready.send(id.to_string());
    }

    /// Looks up an image by placement ID. Decoding the PNG happens after taking it out here (outside the lock)
    pub fn image_data(&self, id: &str) -> Option<Arc<StoredImage>> {
        self.state().image_data.get(id)
    }

    /// Placement of the image currently placed
    pub fn image(&self) -> Option<ImagePlacement> {
        self.state().image.clone()
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

    fn insert(id: &str, index: usize) -> CanvasOp {
        CanvasOp::Insert {
            stroke: stroke(id),
            index,
        }
    }

    fn ids(canvas: &Canvas) -> Vec<String> {
        canvas
            .snapshot()
            .strokes
            .into_iter()
            .map(|s| s.id)
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
        let applied = canvas.apply(
            Origin::Ai,
            CanvasOp::Remove {
                ids: vec!["x".into(), "a".into()],
            },
        );
        assert_eq!(
            applied,
            Some(CanvasOp::Remove {
                ids: vec!["a".into()]
            })
        );
        let none = CanvasOp::Remove {
            ids: vec!["x".into()],
        };
        assert_eq!(canvas.apply(Origin::Ai, none), None);
        assert_eq!(canvas.snapshot().revision, 2);
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
    fn image_ops_keep_a_single_image() {
        let canvas = Canvas::new();
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::RemoveImage),
            None,
            "画像が無ければ無視"
        );
        let set = CanvasOp::SetImage {
            image: placement("a"),
        };
        assert_eq!(canvas.apply(Origin::Ai, set.clone()), Some(set.clone()));
        assert_eq!(canvas.apply(Origin::Ai, set), None, "同じ配置なら変更なし");
        let moved = CanvasOp::SetImage {
            image: ImagePlacement {
                x: 9.0,
                ..placement("a")
            },
        };
        assert_eq!(canvas.apply(Origin::Ai, moved.clone()), Some(moved));
        // One image at most for now, so placing a different ID replaces it
        let swapped = CanvasOp::SetImage {
            image: placement("b"),
        };
        assert_eq!(canvas.apply(Origin::Ai, swapped.clone()), Some(swapped));
        assert_eq!(canvas.snapshot().image, Some(placement("b")));
        assert_eq!(
            canvas.apply(Origin::Ai, CanvasOp::RemoveImage),
            Some(CanvasOp::RemoveImage)
        );
        assert_eq!(canvas.snapshot().image, None);
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
    fn put_image_data_notifies_subscribers() {
        let canvas = Canvas::new();
        let mut ready = canvas.subscribe_images();
        canvas.put_image_data("a", vec![1]);
        assert_eq!(ready.try_recv().unwrap(), "a");
    }
}
