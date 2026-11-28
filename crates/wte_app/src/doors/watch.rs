//! The watched folder's worker (stickers S2; STICKER_PLAN 7 "S2" (b); ARCHITECTURE 8.18).
//!
//! One thread per watched folder, started by the app when a folder is watched and stopped when the app drops the
//! [`FolderWatcher`] (the thread sees its channel close and ends). It polls the folder with `read_dir` and
//! `metadata` - every second, at once on a `Poke`, every 250 ms while a change settles - and follows the `.png` files
//! directly in it (no subfolders; at most [`MAX_FOLDER_PICTURES`], by name). No directory handles, no FFI.
//!
//! A file whose size or time changed is **settled** before it is reported, as the link watcher does (5.6): two
//! consecutive polls must agree, the bytes read must have that size and decode as a picture (PhotoCraft's Generate >
//! Image Assets writes in place, so a half-written file can be seen). After 30 s without a decodable file: one
//! `Unreadable` event, then quiet until the file changes again. A settled file whose size and CRC-32 equal what the
//! app last applied (`known`) only updates the "last seen" stamp: reopening a project reads nothing again. Every
//! repaint request is counted (`FolderWatcher::start`'s counter): with nothing changing it does not move.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// At most this many pictures of one folder are followed (the first ones by name); a folder with more says so once.
pub const MAX_FOLDER_PICTURES: usize = 256;
/// The decode limit of a stamp picture (`Document::add_image_blob`).
const PICTURE_SIDE: u32 = 8192;
/// A picture file larger than this is not read (it says so once).
pub const MAX_FOLDER_FILE_BYTES: u64 = 64 << 20;

/// What the worker reports.
#[derive(Debug)]
pub enum FolderEvent {
    /// A picture settled with new content that decodes.
    Changed { name: String, bytes: Arc<[u8]>, crc: u32 },
    /// A picture did not become decodable within `FolderTimes::give_up`, or is too large: `why` says which.
    Unreadable { name: String, why: String },
    /// The folder is gone (once, until it is back).
    Missing,
    /// The folder is back after `Missing`.
    Back,
    /// The folder holds more than `MAX_FOLDER_PICTURES` pictures (once).
    TooMany { count: usize },
}

/// The worker's timing (tests shorten it).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FolderTimes {
    pub poll: Duration,
    pub settle: Duration,
    pub give_up: Duration,
}

impl Default for FolderTimes {
    fn default() -> FolderTimes {
        FolderTimes { poll: Duration::from_millis(1000), settle: Duration::from_millis(250), give_up: Duration::from_secs(30) }
    }
}

/// The content the app last applied per file name: (size, CRC-32).
pub type Known = HashMap<String, (u64, u32)>;

enum Msg {
    Poke,
    /// Forget `known` and what was seen: every picture is read and reported again ("Read again").
    Rescan,
}

/// The running worker of one folder (dropping it stops the thread).
pub struct FolderWatcher {
    folder: PathBuf,
    tx: Sender<Msg>,
    events: Receiver<FolderEvent>,
    alive: Arc<AtomicBool>,
}

impl FolderWatcher {
    /// Starts the thread for `folder`. `repaint` asks the UI for a frame; `repaints` counts every request.
    pub fn start(folder: PathBuf, known: Known, repaint: Arc<dyn Fn() + Send + Sync>, repaints: Arc<AtomicU64>, times: FolderTimes) -> FolderWatcher {
        let (tx, rx) = std::sync::mpsc::channel();
        let (etx, events) = std::sync::mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let worker = Worker {
            folder: folder.clone(),
            known,
            files: BTreeMap::new(),
            missing: false,
            too_many: false,
            times,
            events: etx,
            repaint,
            repaints,
        };
        let flag = Arc::clone(&alive);
        let spawned = std::thread::Builder::new().name("wte-folder-watcher".to_owned()).spawn(move || {
            worker.run(rx);
            flag.store(false, Ordering::SeqCst);
        });
        if let Err(e) = spawned {
            log::error!("the folder watcher could not start: {e}");
            alive.store(false, Ordering::SeqCst);
        }
        FolderWatcher { folder, tx, events, alive }
    }

    /// The folder this worker follows.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Polls at once.
    pub fn poke(&self) {
        let _ = self.tx.send(Msg::Poke);
    }

    /// Reads every picture again and reports each one.
    pub fn rescan(&self) {
        let _ = self.tx.send(Msg::Rescan);
    }

    /// The events reported since the last call.
    pub fn drain(&self) -> Vec<FolderEvent> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(e) => out.push(e),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// False once the thread has ended.
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

/// (size, modification time in ns).
type Meta = (u64, Option<u64>);

#[derive(Clone, Debug, Default)]
struct FileState {
    last_seen: Option<Meta>,
    settle: Option<(Meta, Instant)>,
}

struct Worker {
    folder: PathBuf,
    known: Known,
    files: BTreeMap<String, FileState>,
    missing: bool,
    too_many: bool,
    times: FolderTimes,
    events: Sender<FolderEvent>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    repaints: Arc<AtomicU64>,
}

/// True for a file name the folder watch follows: `*.png` (any case), not hidden, not a temp file of a writer.
pub fn is_picture_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".png") && lower.len() > 4 && !lower.starts_with('.') && !lower.starts_with('~')
}

fn meta_of(m: &std::fs::Metadata) -> Meta {
    (m.len(), m.modified().ok().and_then(wte_core::link::unix_ns))
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        let mut first = true;
        loop {
            if !first {
                let settling = self.files.values().any(|f| f.settle.is_some());
                let timeout = if settling { self.times.settle } else { self.times.poll };
                match rx.recv_timeout(timeout) {
                    Ok(Msg::Poke) | Err(RecvTimeoutError::Timeout) => {}
                    Ok(Msg::Rescan) => {
                        self.known.clear();
                        self.files.clear();
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            first = false;
            let sent = self.poll();
            if sent > 0 {
                self.repaints.fetch_add(1, Ordering::SeqCst);
                (self.repaint)();
            }
        }
    }

    fn send(&self, e: FolderEvent) -> usize {
        usize::from(self.events.send(e).is_ok())
    }

    /// One look at the folder; the number of events sent.
    fn poll(&mut self) -> usize {
        let mut sent = 0;
        let listing = match std::fs::read_dir(&self.folder) {
            Ok(l) => l,
            Err(_) => {
                if !self.missing {
                    self.missing = true;
                    self.files.clear();
                    sent += self.send(FolderEvent::Missing);
                }
                return sent;
            }
        };
        if self.missing {
            self.missing = false;
            sent += self.send(FolderEvent::Back);
        }
        let mut found: BTreeMap<String, Meta> = BTreeMap::new();
        for entry in listing.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if !is_picture_name(&name) {
                continue;
            }
            let Ok(m) = entry.metadata() else { continue };
            if m.is_file() {
                found.insert(name, meta_of(&m));
            }
        }
        if found.len() > MAX_FOLDER_PICTURES {
            if !self.too_many {
                self.too_many = true;
                sent += self.send(FolderEvent::TooMany { count: found.len() });
            }
            found = found.into_iter().take(MAX_FOLDER_PICTURES).collect();
        }
        // Files that are gone are forgotten (one that comes back is read again).
        self.files.retain(|name, _| found.contains_key(name));
        for (name, meta) in found {
            sent += self.poll_file(name, meta);
        }
        sent
    }

    fn poll_file(&mut self, name: String, meta: Meta) -> usize {
        let st = self.files.entry(name.clone()).or_default();
        if st.settle.is_none() && st.last_seen == Some(meta) {
            return 0;
        }
        match st.settle {
            Some((m, _)) if m == meta => {}
            Some((_, since)) => {
                st.settle = Some((meta, since));
                return 0;
            }
            None => {
                st.settle = Some((meta, Instant::now()));
                return 0;
            }
        }
        let since = st.settle.map_or_else(Instant::now, |(_, s)| s);
        if meta.0 > MAX_FOLDER_FILE_BYTES {
            st.settle = None;
            st.last_seen = Some(meta);
            let why = format!("it is larger than {} MB", MAX_FOLDER_FILE_BYTES >> 20);
            return self.send(FolderEvent::Unreadable { name, why });
        }
        let path = self.folder.join(&name);
        let bytes = match std::fs::read(&path) {
            Ok(b) if b.len() as u64 == meta.0 => b,
            // Changed under the read, or held by its writer: keep settling.
            _ => return 0,
        };
        let crc = crate::link::watch::crc(&bytes);
        let st = self.files.entry(name.clone()).or_default();
        if self.known.get(&name) == Some(&(meta.0, crc)) {
            st.settle = None;
            st.last_seen = Some(meta);
            return 0;
        }
        match wte_core::decode_image(&bytes, Some(&name), PICTURE_SIDE) {
            Ok(_) => {
                st.settle = None;
                st.last_seen = Some(meta);
                self.known.insert(name.clone(), (meta.0, crc));
                self.send(FolderEvent::Changed { name, bytes: Arc::from(bytes), crc })
            }
            Err(_) if since.elapsed() < self.times.give_up => 0,
            Err(e) => {
                st.settle = None;
                st.last_seen = Some(meta);
                self.send(FolderEvent::Unreadable { name, why: e.to_string() })
            }
        }
    }
}
