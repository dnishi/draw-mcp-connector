//! Writes the canvas state to the data folder and reads it back at launch.
//!
//! The files always match the current state: what Clear, Delete Sheet, or Delete All Sheets erases is
//! also removed from the files. No copies of older states (generations) are kept.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::warn;

use crate::app::App;
use crate::canvas::Canvas;
use crate::config;
use crate::lock::lock;
use crate::protocol::{MAX_SHEETS, Sheet};
use crate::render::StoredImage;

/// Version of the file format. Currently 1
const FORMAT: u32 = 1;

/// State read at launch
pub struct Saved {
    pub revision: u64,
    pub current: String,
    pub sheets: Vec<Sheet>,
    /// Bytes of the placed images (image ID, bytes). Images whose files were missing are not included
    pub images: Vec<(String, Vec<u8>)>,
}

/// Type for reading and writing canvas.json (only inside store.rs; uses protocol.rs's Sheet as is)
#[derive(Debug, Serialize, Deserialize)]
struct FileFormat {
    format: u32,
    revision: u64,
    current: String,
    sheets: Vec<Sheet>,
}

/// Image IDs that can be file names: 1 to 64 ASCII letters, digits, `-`, or `_`.
/// IDs are strings from the page, and using them in paths could write outside the folder via `../` and such,
/// so the bytes of images with other IDs are not saved (their placement stays in canvas.json)
fn valid_image_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
}

/// Saves the canvas state to a folder. So that two callers never write at once,
/// the whole write is guarded by one lock
pub struct Store {
    mutex: Mutex<Inner>,
}

struct Inner {
    dir: PathBuf,
    /// Images written (image ID, the `Arc` returned by `Canvas::image_data`).
    /// Rewritten when the `Arc` for the same ID is a different one (the contents arrived again), not when it is the same
    written: HashMap<String, Arc<StoredImage>>,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            mutex: Mutex::new(Inner {
                dir,
                written: HashMap::new(),
            }),
        }
    }

    /// Reads canvas.json. None if it does not exist. If it is broken, renames it to canvas.json.broken and returns None
    pub fn load(&self) -> Option<Saved> {
        let inner = lock(&self.mutex);
        let path = inner.dir.join("canvas.json");
        let bytes = match std::fs::read(&path) {
            // Without it, start blank (nothing to keep either)
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                warn!("canvas.json を読めない: {error}");
                mark_broken(&path);
                return None;
            }
            Ok(bytes) => bytes,
        };
        let file = match serde_json::from_slice::<FileFormat>(&bytes) {
            Ok(file) => file,
            Err(error) => {
                warn!("canvas.json が JSON ではない: {error}");
                mark_broken(&path);
                return None;
            }
        };
        if !rules_ok(&file) {
            warn!(
                "canvas.json が決まりに合わない(format: {}, 紙: {} 枚)",
                file.format,
                file.sheets.len()
            );
            mark_broken(&path);
            return None;
        }
        let images = load_images(&inner.dir, &file.sheets);
        Some(Saved {
            revision: file.revision,
            current: file.current,
            sheets: file.sheets,
            images,
        })
    }

    /// Writes the current state. Failures are logged with warn and dropped (the app keeps running and writes again on the next change)
    pub fn save(&self, canvas: &Canvas) {
        let mut inner = lock(&self.mutex);
        let snapshot = canvas.snapshot();
        let file = FileFormat {
            format: FORMAT,
            revision: snapshot.revision,
            current: snapshot.current,
            sheets: snapshot.sheets,
        };
        // Write the images before canvas.json (so that canvas.json never points to an image not yet written)
        // Write the strokes even if the images cannot be written (when reading, the placement stays without the image file)
        if let Err(error) = write_images(&mut inner, canvas, &file) {
            warn!("画像のフォルダを作れない: {error}");
        }
        let json = match serde_json::to_string(&file) {
            Ok(json) => json,
            Err(error) => {
                warn!("canvas.json を JSON にできない: {error}");
                return;
            }
        };
        if let Err(error) = write_tmp_and_rename(&inner.dir.join("canvas.json"), json.as_bytes()) {
            warn!("canvas.json を書き込めない: {error}");
            return;
        }
        // Delete the files of images no longer placed on any sheet
        if let Err(error) = remove_unplaced_images(&mut inner, &file) {
            warn!("置かなくなった画像のファイルを消せない: {error}");
        }
    }

    /// Records the images placed on some sheet that have contents as written.
    /// So that the first save after launch does not rewrite the images just read from the files
    pub fn remember_images(&self, canvas: &Canvas) {
        let mut inner = lock(&self.mutex);
        for id in canvas.placed_image_ids() {
            if let Some(image) = canvas.image_data(&id) {
                inner.written.insert(id, image);
            }
        }
    }
}

/// On a change, waits SAVE_DELAY and then writes. Changes during the wait are written together once
pub async fn run(app: Arc<App>, store: Arc<Store>) {
    let (_snapshot, mut changes) = app.canvas.subscribe();
    let mut images = app.canvas.subscribe_images();
    loop {
        let closed = tokio::select! {
            // Lagged also counts as a change (we only missed the messages)
            change = changes.recv() => {
                matches!(change, Err(broadcast::error::RecvError::Closed))
            }
            image = images.recv() => {
                matches!(image, Err(broadcast::error::RecvError::Closed))
            }
        };
        if closed {
            break;
        }
        tokio::time::sleep(config::SAVE_DELAY).await;
        // Discard the changes queued on both receivers during the wait (what gets written is the current state)
        while changes.try_recv().is_ok() {}
        while images.try_recv().is_ok() {}
        // Writing files blocks, so do it on another thread
        let (app, store) = (app.clone(), store.clone());
        let saved = tokio::task::spawn_blocking(move || store.save(&app.canvas)).await;
        if let Err(error) = saved {
            warn!("キャンバスを保存できない: {error}");
        }
    }
}

/// Keeps a broken file by renaming it to canvas.json.broken (replacing any existing one)
fn mark_broken(path: &Path) {
    let mut broken = path.as_os_str().to_os_string();
    broken.push(".broken");
    if let Err(error) = std::fs::rename(path, broken) {
        warn!("{path:?} を .broken に名前を変えられない: {error}");
    }
}

/// Whether the contents follow the rules: 1 to MAX_SHEETS sheets, no duplicate sheet IDs, and current is one of the sheets
fn rules_ok(file: &FileFormat) -> bool {
    let unique_ids = file
        .sheets
        .iter()
        .map(|sheet| sheet.id.as_str())
        .collect::<HashSet<_>>();
    file.format == FORMAT
        && (1..=MAX_SHEETS).contains(&file.sheets.len())
        && unique_ids.len() == file.sheets.len()
        && file.sheets.iter().any(|sheet| sheet.id == file.current)
}

/// Reads the bytes of the placed images. Missing files are left out (the placement stays in canvas.json)
fn load_images(dir: &Path, sheets: &[Sheet]) -> Vec<(String, Vec<u8>)> {
    let mut images = Vec::new();
    let mut seen = HashSet::new();
    for sheet in sheets {
        let Some(image) = &sheet.image else {
            continue;
        };
        if !seen.insert(image.id.clone()) || !valid_image_id(&image.id) {
            continue;
        }
        let path = dir.join("images").join(format!("{}.png", image.id));
        if let Ok(bytes) = std::fs::read(&path) {
            images.push((image.id.clone(), bytes));
        }
    }
    images
}

/// Writes the bytes of the placed images. Images that cannot be written are skipped with warn (rewritten on the next save).
/// Returns an error only when the images folder cannot be created
fn write_images(inner: &mut Inner, canvas: &Canvas, file: &FileFormat) -> io::Result<()> {
    let images = inner.dir.join("images");
    std::fs::create_dir_all(&images)?;
    let mut placed = HashSet::new();
    for sheet in &file.sheets {
        if let Some(image) = &sheet.image {
            placed.insert(image.id.clone());
        }
    }
    for id in &placed {
        if !valid_image_id(id) {
            warn!("画像の ID {id:?} が条件に合わないため、バイト列を保存しない");
            continue;
        }
        // Do not write bytes that have not arrived yet (they are written on the next save after they arrive)
        let Some(image) = canvas.image_data(id) else {
            continue;
        };
        // The same contents are already written
        if inner
            .written
            .get(id)
            .is_some_and(|written| Arc::ptr_eq(written, &image))
        {
            continue;
        }
        let path = images.join(format!("{id}.png"));
        if let Err(error) = write_tmp_and_rename(&path, image.bytes()) {
            warn!("画像 {id} を書き込めない: {error}");
            continue;
        }
        inner.written.insert(id.clone(), image);
    }
    Ok(())
}

/// Deletes from `images/` the files of images no longer placed on any sheet
fn remove_unplaced_images(inner: &mut Inner, file: &FileFormat) -> io::Result<()> {
    let images = inner.dir.join("images");
    let placed = file
        .sheets
        .iter()
        .filter_map(|sheet| sheet.image.as_ref())
        .map(|image| image.id.as_str())
        .collect::<HashSet<_>>();
    for entry in std::fs::read_dir(&images)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".png") else {
            continue;
        };
        if !placed.contains(id) {
            std::fs::remove_file(entry.path())?;
            // If placed again, rewrite it even with the same `Arc`
            inner.written.remove(id);
        }
    }
    Ok(())
}

/// Writes to `.tmp` and then renames it (if writing is cut off, the previous file remains).
/// Replaces an existing file too (also on Windows)
fn write_tmp_and_rename(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::app::App;
    use crate::canvas::{Canvas, Origin};
    use crate::protocol::{
        Author, CanvasOp, INITIAL_SHEET_ID, ImagePlacement, Stroke, StrokePoint,
    };

    /// Number of temporary folders already made in this test process
    static CREATED: AtomicUsize = AtomicUsize::new(0);

    /// A temporary folder with a different name per test (process ID and a counter). Deleted at the end of the test
    pub(crate) fn temp_dir() -> PathBuf {
        let number = CREATED.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bridge-store-{}-{number}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

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

    fn placement(id: &str) -> ImagePlacement {
        ImagePlacement {
            id: id.into(),
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        }
    }

    /// A canvas with two sheets (the current one is "b"): a stroke on the first, an image on the second
    pub(crate) fn canvas_with_image(image_id: &str, bytes: &[u8]) -> Canvas {
        let canvas = Canvas::new();
        canvas.apply(
            Origin::Ai,
            CanvasOp::Insert {
                sheet: INITIAL_SHEET_ID.into(),
                stroke: stroke("s"),
                index: 0,
            },
        );
        canvas.apply(
            Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        canvas.apply(Origin::Ai, CanvasOp::SelectSheet { id: "b".into() });
        canvas.apply(
            Origin::Ai,
            CanvasOp::SetImage {
                sheet: "b".into(),
                image: placement(image_id),
            },
        );
        canvas.put_image_data(image_id, bytes.to_vec());
        canvas
    }

    #[test]
    fn saving_then_loading_returns_the_same_state() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        let canvas = canvas_with_image("img", &[9, 9]);
        // Bytes of images that are not placed are not saved
        canvas.put_image_data("not-placed", vec![1]);
        store.save(&canvas);

        assert!(dir.join("canvas.json").exists());
        assert_eq!(std::fs::read(dir.join("images/img.png")).unwrap(), [9, 9]);

        let saved = store.load().expect("読めるはず");
        let snapshot = canvas.snapshot();
        assert_eq!(saved.revision, snapshot.revision);
        assert_eq!(saved.current, snapshot.current);
        assert_eq!(saved.sheets, snapshot.sheets);
        assert_eq!(saved.images, vec![("img".into(), vec![9, 9])]);

        cleanup(&dir);
    }

    #[test]
    fn deleting_a_sheet_deletes_its_image_file() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        let canvas = canvas_with_image("img", &[1]);
        store.save(&canvas);
        let image_path = dir.join("images/img.png");
        assert!(image_path.exists());

        canvas.apply(Origin::Ai, CanvasOp::RemoveSheet { id: "b".into() });
        store.save(&canvas);

        assert!(!image_path.exists(), "紙が消えたらその画像も消える");
        assert!(dir.join("canvas.json").exists());

        cleanup(&dir);
    }

    #[test]
    fn a_written_image_is_not_rewritten_until_the_content_changes() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        let canvas = canvas_with_image("img", b"first");
        store.save(&canvas);
        let image_path = dir.join("images/img.png");

        // Checks that the file is not rewritten: a rewrite would overwrite it and restore the contents,
        // and without a rewrite the contents changed from outside stay
        std::fs::write(&image_path, b"touched").unwrap();
        store.save(&canvas);
        assert_eq!(
            std::fs::read(&image_path).unwrap(),
            b"touched",
            "同じ中身なら書き直さない"
        );

        // When the contents for the same ID are replaced, the file gets the new contents
        canvas.put_image_data("img", b"second".to_vec());
        store.save(&canvas);
        assert_eq!(std::fs::read(&image_path).unwrap(), b"second");

        cleanup(&dir);
    }

    #[test]
    fn an_invalid_image_id_is_not_written_but_kept_in_canvas_json() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        let canvas = canvas_with_image("../x", &[1]);
        store.save(&canvas);

        assert_eq!(
            dir.join("images").read_dir().unwrap().count(),
            0,
            "画像のファイルを作らない"
        );
        let saved = store.load().expect("読めるはず");
        assert_eq!(saved.sheets[1].image, Some(placement("../x")));
        assert!(saved.images.is_empty());

        cleanup(&dir);
    }

    #[test]
    fn loading_without_canvas_json_is_none() {
        let dir = temp_dir();
        assert!(Store::new(dir.clone()).load().is_none());

        cleanup(&dir);
    }

    #[test]
    fn a_broken_canvas_json_is_renamed_to_broken() {
        let sheet = r#"{"id":"sheet-1","strokes":[],"image":null}"#;
        let cases = [
            ("JSON でない", "壊れた中身"),
            (
                "format が 2",
                &format!(
                    "{{\"format\":2,\"revision\":0,\"current\":\"sheet-1\",\"sheets\":[{sheet}]}}"
                ),
            ),
            (
                "紙が 0 枚",
                "{\"format\":1,\"revision\":0,\"current\":\"sheet-1\",\"sheets\":[]}",
            ),
            (
                "current が一覧に無い",
                &format!(
                    "{{\"format\":1,\"revision\":0,\"current\":\"elsewhere\",\"sheets\":[{sheet}]}}"
                ),
            ),
            (
                "紙の ID が重なる",
                &format!(
                    "{{\"format\":1,\"revision\":0,\"current\":\"sheet-1\",\"sheets\":[{sheet},{sheet}]}}"
                ),
            ),
        ];
        for (name, content) in cases {
            let dir = temp_dir();
            std::fs::write(dir.join("canvas.json"), content).unwrap();
            let saved = Store::new(dir.clone()).load();
            assert!(saved.is_none(), "{name}");
            assert!(!dir.join("canvas.json").exists());
            assert!(dir.join("canvas.json.broken").exists());
            cleanup(&dir);
        }
    }

    #[test]
    fn a_broken_canvas_json_replaces_an_existing_broken_file() {
        let dir = temp_dir();
        std::fs::write(dir.join("canvas.json.broken"), "古い").unwrap();
        std::fs::write(dir.join("canvas.json"), "壊れた").unwrap();
        assert!(Store::new(dir.clone()).load().is_none());
        assert_eq!(
            std::fs::read_to_string(dir.join("canvas.json.broken")).unwrap(),
            "壊れた"
        );

        cleanup(&dir);
    }

    #[test]
    fn a_missing_image_file_is_skipped_but_the_placement_stays() {
        let dir = temp_dir();
        let store = Store::new(dir.clone());
        let canvas = canvas_with_image("img", &[1]);
        store.save(&canvas);
        std::fs::remove_file(dir.join("images/img.png")).unwrap();

        let saved = store.load().expect("読めるはず");
        assert!(saved.images.is_empty(), "バイト列は無い");
        assert_eq!(saved.sheets[1].image, Some(placement("img")), "配置は残る");

        cleanup(&dir);
    }

    #[test]
    fn a_loaded_image_is_not_rewritten_by_the_first_save() {
        let dir = temp_dir();
        let canvas = canvas_with_image("img", b"first");
        Store::new(dir.clone()).save(&canvas);
        let image_path = dir.join("images/img.png");

        // Restart: load, and record the loaded images as written
        let store = Store::new(dir.clone());
        let saved = store.load().expect("読めるはず");
        let canvas = Canvas::restored(saved);
        store.remember_images(&canvas);
        std::fs::write(&image_path, b"touched").unwrap();
        store.save(&canvas);

        // Checks that it is not rewritten: a rewrite would overwrite it and restore the contents
        assert_eq!(
            std::fs::read(&image_path).unwrap(),
            b"touched",
            "読んだばかりの画像は書き直さない"
        );

        cleanup(&dir);
    }

    /// run waits for changes and writes. spawn_blocking actually runs instead of following the paused clock,
    /// so wait on the file for a while to see that it was written
    #[tokio::test]
    async fn run_writes_a_new_stroke_after_the_delay() {
        let dir = temp_dir();
        let app = Arc::new(App::with_store(Store::new(dir.clone())));
        let store = app.store.clone().expect("store が無い");
        let runner = tokio::spawn(run(app.clone(), store));
        // A change applied before run is listening is missed, so wait for run to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        app.canvas.apply(
            Origin::Ai,
            CanvasOp::Insert {
                sheet: INITIAL_SHEET_ID.into(),
                stroke: stroke("s"),
                index: 0,
            },
        );

        let json = dir.join("canvas.json");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline
            && !std::fs::read_to_string(&json).is_ok_and(|json| json.contains("\"s\""))
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            std::fs::read_to_string(&json).is_ok_and(|json| json.contains("\"s\"")),
            "SAVE_DELAY の後に筆跡が canvas.json に入る"
        );

        runner.abort();
        cleanup(&dir);
    }
}
