//! The live PNG copy (stickers S2; STICKER_PLAN 7 "S2" (a); ARCHITECTURE 8.18): per sticker or raster paint layer,
//! "Keep a PNG copy up to date" writes the finished picture into the project's links folder, again
//! [`LIVE_COPY_DELAY`] after its last change, for PhotoCraft's File > Place Linked.
//!
//! The copy follows the sticker's picture key (`sticker::canvas::sticker_key`: layers by value, pixels by version) or
//! the layer's pixel version, plus the file's path (a Save As moves the links folder: the copy is written into the new
//! one). At most one write runs at a time, on a thread: the picture is drawn there (`Document::sticker_picture_job`;
//! a paint layer's buffer is an `Arc`), encoded as PNG, compared with the file (the same bytes are not written again,
//! so reopening a project leaves the file and its time alone) and written atomically (a temp file and a rename, the
//! core's `write_atomic`). A write that fails (the file held by another program) is tried again after
//! [`RETRY_DELAY`]. Nothing waits while a stroke, a drag or a pending Preview edit is open.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use wte_core::{Document, LayerId, LayerKind, PixelBuf, PngLevel, StickerId};

use crate::state::AppState;
use crate::sticker::canvas::CanvasKey;

/// A copy is written this long after the last change of its picture (S2: 500 ms).
pub const LIVE_COPY_DELAY: Duration = Duration::from_millis(500);
/// A failed write is tried again after this.
pub const RETRY_DELAY: Duration = Duration::from_secs(2);

/// What a live copy follows.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveTarget {
    Sticker(StickerId),
    Layer(LayerId),
}

/// What the copy was made from, by value (no buffer kept).
#[derive(Clone, Debug, PartialEq)]
enum Source {
    Sticker(CanvasKey),
    Layer { version: u64, size: [u32; 2] },
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    source: Source,
    path: PathBuf,
}

/// What the Sticker and Layers tabs say about a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveStatus {
    /// Not written yet in this session (or changed since): waiting for the delay.
    Waiting,
    Writing,
    UpToDate,
    /// The project has no file yet: the links folder is beside it.
    NeedsSave,
    /// The sticker or layer is not in the document (deleted; an undo can bring it back).
    Gone,
    /// The last write failed (tried again).
    Failed(String),
}

/// One live copy.
#[derive(Clone, Debug)]
pub struct LiveEntry {
    /// The file name in the links folder (fixed when the copy is turned on, so a renamed sticker keeps its file and
    /// PhotoCraft's placed link).
    pub file: String,
    pub status: LiveStatus,
    /// The content the file holds (written, or found equal).
    written: Option<Key>,
    /// The current content when it differs from `written`, and since when it is the current one.
    changed: Option<(Key, Instant)>,
    /// Written at once (no delay): a copy just turned on or restored with the project.
    at_once: bool,
    retry_at: Option<Instant>,
}

impl LiveEntry {
    pub fn new(file: String) -> LiveEntry {
        LiveEntry { file, status: LiveStatus::Waiting, written: None, changed: None, at_once: true, retry_at: None }
    }
}

/// Counts for the tests and scenarios.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveCounters {
    /// Files written.
    pub writes: u64,
    /// Jobs that found the file already holding the picture.
    pub same: u64,
    pub failures: u64,
    /// Jobs started.
    pub jobs: u64,
}

enum Outcome {
    Written,
    Same,
}

struct Job {
    target: LiveTarget,
    key: Key,
    rx: Receiver<Result<Outcome, String>>,
}

/// The live copies of the open document.
pub struct LiveCopies {
    pub entries: BTreeMap<LiveTarget, LiveEntry>,
    job: Option<Job>,
    pub delay: Duration,
    /// How long a failed write waits before it is tried again (`RETRY_DELAY`; tests shorten it).
    pub retry: Duration,
    pub counters: LiveCounters,
}

impl Default for LiveCopies {
    fn default() -> LiveCopies {
        LiveCopies { entries: BTreeMap::new(), job: None, delay: LIVE_COPY_DELAY, retry: RETRY_DELAY, counters: LiveCounters::default() }
    }
}

impl LiveCopies {
    /// True while a write runs.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The document changed: every copy and a running job belong to the old one (the job's thread still finishes
    /// its file).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.job = None;
    }
}

/// How a live copy is saved with the project (`Document::ui_state`, key `live_copies`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLiveCopy {
    /// `"sticker"` or `"layer"`.
    pub kind: String,
    pub id: u64,
    pub file: String,
}

/// The saved form of every copy whose sticker or raster layer is in `doc`.
pub fn saved(copies: &LiveCopies, doc: &Document) -> Vec<SavedLiveCopy> {
    copies
        .entries
        .iter()
        .filter(|(t, _)| exists(doc, **t))
        .map(|(t, e)| match t {
            LiveTarget::Sticker(s) => SavedLiveCopy { kind: "sticker".to_owned(), id: s.0, file: e.file.clone() },
            LiveTarget::Layer(l) => SavedLiveCopy { kind: "layer".to_owned(), id: l.0, file: e.file.clone() },
        })
        .collect()
}

/// The copies of a loaded project (unknown kinds, unsafe file names and targets that are not in `doc` are left out).
pub fn restore(copies: &mut LiveCopies, saved: &[SavedLiveCopy], doc: &Document) {
    for s in saved {
        let target = match s.kind.as_str() {
            "sticker" => LiveTarget::Sticker(StickerId(s.id)),
            "layer" => LiveTarget::Layer(LayerId(s.id)),
            other => {
                log::warn!("ignoring a live copy of an unknown kind \"{other}\"");
                continue;
            }
        };
        if !safe_file_name(&s.file) || !exists(doc, target) {
            log::warn!("ignoring the live copy {:?}", s);
            continue;
        }
        copies.entries.insert(target, LiveEntry::new(s.file.clone()));
    }
}

/// A plain file name ending in `.png` (no folder part), as WTE makes them.
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.to_lowercase().ends_with(".png")
        && !name.contains(['/', '\\', ':'])
        && name != ".."
        && Path::new(name).file_name().is_some_and(|n| n == name)
}

/// True when the copy's sticker, or raster paint layer, is in `doc`.
pub fn exists(doc: &Document, target: LiveTarget) -> bool {
    match target {
        LiveTarget::Sticker(s) => doc.sticker(s).is_some(),
        LiveTarget::Layer(l) => doc.layer(l).is_some_and(|x| x.is_raster()),
    }
}

/// The file name of a new copy: the link file's name with `.png` (`<name>_k<id>.png`, `<name>_l<id>.png`).
pub fn file_name_for(doc: &Document, target: LiveTarget) -> Option<String> {
    let psd = match target {
        LiveTarget::Sticker(s) => wte_core::link::link_file_name(&doc.sticker(s)?.name, s),
        LiveTarget::Layer(l) => wte_core::link::paint_link_file_name(&doc.layer(l)?.name, l),
    };
    Some(format!("{}.png", psd.strip_suffix(".psd").unwrap_or(&psd)))
}

/// The copy's path (`None` for a project that has no file).
pub fn path_of(state: &AppState, target: LiveTarget) -> Option<PathBuf> {
    let project = state.session.as_ref()?.path.as_deref()?;
    let e = state.doors.live.entries.get(&target)?;
    Some(wte_core::link::links_folder(project).join(&e.file))
}

fn source_of(doc: &Document, target: LiveTarget) -> Option<Source> {
    match target {
        LiveTarget::Sticker(s) => Some(Source::Sticker(crate::sticker::canvas::sticker_key(s, doc.sticker(s)?))),
        LiveTarget::Layer(l) => {
            let layer = doc.layer(l).filter(|x| x.is_raster())?;
            Some(Source::Layer { version: layer.version(), size: doc.layer_texture_size(l)? })
        }
    }
}

/// What the job's thread draws.
enum Picture {
    Sticker(wte_core::StickerPictureJob),
    Layer(Arc<PixelBuf>, [u32; 2]),
}

impl Picture {
    fn draw(self) -> PixelBuf {
        match self {
            Picture::Sticker(job) => job.run(),
            Picture::Layer(buf, [w, h]) if buf.width == w && buf.height == h => Arc::unwrap_or_clone(buf),
            // An unallocated (never painted) layer is transparent.
            Picture::Layer(_, [w, h]) => PixelBuf::new(w, h),
        }
    }
}

/// The picture's PNG bytes; then the file is left alone when it holds them already, else written atomically.
fn write_copy(picture: Picture, path: &Path) -> Result<Outcome, String> {
    let p = picture.draw();
    if p.width == 0 || p.height == 0 {
        return Err("the picture is empty".to_owned());
    }
    let bytes = wte_core::encode_png(p.width, p.height, &p.data, PngLevel::Balanced).map_err(|e| e.to_string())?;
    if std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64) && std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(Outcome::Same);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    wte_core::project::atomic::write_atomic::<std::io::Error>(path, false, |f| f.write_all(&bytes), |_| Ok(())).map_err(|e| e.to_string())?;
    Ok(Outcome::Written)
}

/// Every frame (`doors::pump`): takes a finished write, then starts the next due one (at most one runs). `busy`: an
/// interaction is open (nothing starts). Returns how long until the next copy is due, when one waits.
pub fn pump(state: &mut AppState, ctx: &egui::Context, busy: bool) -> Option<Duration> {
    take_finished(state);
    let session = state.session.as_ref()?;
    let doc = &session.doc;
    let folder = session.path.as_deref().map(wte_core::link::links_folder);
    let now = Instant::now();
    let live = &mut state.doors.live;
    let delay = live.delay;
    let running = live.job.as_ref().map(|j| (j.target, j.key.clone()));
    let mut wake: Option<Duration> = None;
    let mut due: Option<(LiveTarget, Key)> = None;
    for (target, e) in live.entries.iter_mut() {
        let Some(folder) = &folder else {
            e.status = LiveStatus::NeedsSave;
            continue;
        };
        let Some(source) = source_of(doc, *target) else {
            e.status = LiveStatus::Gone;
            e.changed = None;
            continue;
        };
        let key = Key { source, path: folder.join(&e.file) };
        if e.written.as_ref() == Some(&key) {
            e.changed = None;
            e.status = LiveStatus::UpToDate;
            continue;
        }
        if running.as_ref().is_some_and(|(t, k)| t == target && *k == key) {
            e.status = LiveStatus::Writing;
            continue;
        }
        if e.changed.as_ref().is_none_or(|(k, _)| *k != key) {
            e.changed = Some((key.clone(), now));
        }
        if !matches!(e.status, LiveStatus::Failed(_)) {
            e.status = LiveStatus::Waiting;
        }
        let since = e.changed.as_ref().map_or(now, |(_, t)| *t);
        let mut at = if e.at_once { since } else { since + delay };
        if let Some(r) = e.retry_at {
            at = at.max(r);
        }
        if now >= at && due.is_none() {
            due = Some((*target, key));
        } else {
            let left = at.saturating_duration_since(now).max(Duration::from_millis(16));
            wake = Some(wake.map_or(left, |w| w.min(left)));
        }
    }
    let Some((target, key)) = due else { return wake };
    if busy || running.is_some() {
        return Some(wake.map_or(Duration::from_millis(100), |w| w.min(Duration::from_millis(100))));
    }
    start(state, ctx, target, key);
    wake
}

fn start(state: &mut AppState, ctx: &egui::Context, target: LiveTarget, key: Key) {
    let Some(doc) = state.doc_mut() else { return };
    let picture = match target {
        LiveTarget::Sticker(s) => match doc.sticker_picture_job(s, 1.0) {
            Ok(job) => Picture::Sticker(job),
            Err(e) => return failed(state, target, e.to_string()),
        },
        LiveTarget::Layer(l) => {
            let size = doc.layer_texture_size(l);
            match (doc.layer(l).map(|x| &x.kind), size) {
                (Some(LayerKind::Raster(buf)), Some(size)) => Picture::Layer(Arc::clone(buf), size),
                _ => return,
            }
        }
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let path = key.path.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("wte-live-copy".to_owned()).spawn(move || {
        let _ = tx.send(write_copy(picture, &path));
        repaint.request_repaint();
    });
    if let Err(e) = spawned {
        return failed(state, target, e.to_string());
    }
    let live = &mut state.doors.live;
    live.counters.jobs += 1;
    if let Some(e) = live.entries.get_mut(&target) {
        e.status = LiveStatus::Writing;
    }
    live.job = Some(Job { target, key, rx });
}

fn failed(state: &mut AppState, target: LiveTarget, why: String) {
    let live = &mut state.doors.live;
    live.counters.failures += 1;
    let retry = live.retry;
    if let Some(e) = live.entries.get_mut(&target) {
        if !matches!(&e.status, LiveStatus::Failed(w) if *w == why) {
            log::warn!("the PNG copy {} could not be written: {why}", e.file);
        }
        e.status = LiveStatus::Failed(why);
        e.retry_at = Some(Instant::now() + retry);
    }
}

fn take_finished(state: &mut AppState) {
    let live = &mut state.doors.live;
    let Some(job) = &live.job else { return };
    let result = match job.rx.try_recv() {
        Ok(r) => r,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("the writer stopped".to_owned()),
    };
    let Some(job) = live.job.take() else { return };
    match result {
        Ok(outcome) => {
            match outcome {
                Outcome::Written => live.counters.writes += 1,
                Outcome::Same => live.counters.same += 1,
            }
            if let Some(e) = live.entries.get_mut(&job.target) {
                e.written = Some(job.key);
                e.at_once = false;
                e.retry_at = None;
                e.status = LiveStatus::Waiting;
            }
        }
        Err(why) => failed(state, job.target, why),
    }
}

/// The status line of a copy ("PNG copy: Badge_k12.png, up to date").
pub fn status_text(e: &LiveEntry) -> String {
    let what = match &e.status {
        LiveStatus::Waiting => "waiting for changes to settle".to_owned(),
        LiveStatus::Writing => "writing...".to_owned(),
        LiveStatus::UpToDate => "up to date".to_owned(),
        LiveStatus::NeedsSave => "save the project first".to_owned(),
        LiveStatus::Gone => "not written (its picture is not in the project)".to_owned(),
        LiveStatus::Failed(why) => format!("could not be written ({why}); trying again"),
    };
    format!("PNG copy: {}, {what}", e.file)
}
