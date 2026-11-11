//! Canvas state held by the bridge. Changes from both the canvas page and the AI go through here.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::lock::lock;
use crate::protocol::{CanvasOp, PageSize, Stroke, Viewport};

/// Number of changes remembered to return diffs. Diffs from older revisions are returned as the whole canvas
const MAX_LOG_LEN: usize = 10_000;

/// Who made a change. Used to avoid sending a change back to its sender
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
        }
    }
}

pub struct Canvas {
    state: Mutex<State>,
    changes: broadcast::Sender<Change>,
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            state: Mutex::default(),
            changes: broadcast::channel(1024).0,
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

    /// Gets a snapshot of the whole canvas and a receiver for later changes at the same time (so no change in between is missed)
    pub fn subscribe(&self) -> (Vec<Stroke>, broadcast::Receiver<Change>) {
        let state = self.state();
        (state.strokes.clone(), self.changes.subscribe())
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
}
