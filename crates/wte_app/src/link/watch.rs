//! The link watcher (STICKER_PLAN 5.6; stickers P9a; ARCHITECTURE 4.6, 8.16).
//!
//! One worker thread, started when the first link exists and stopped with the last one (the app drops the
//! [`LinkWatcher`]; the thread sees its channel close and ends). It owns the list of watched files (replaced through a
//! channel when links change) and polls `std::fs::metadata` of each: every second, at once on a `Poke` (the window
//! gained focus, "Update now"), every 250 ms while a change settles. No directory handles, no FFI.
//!
//! A file whose size or time changed is **settled** before it is reported: two consecutive polls must agree and
//! `psd::read` must accept the bytes (an editor that writes in place leaves half-written files; `Truncated` just
//! waits). After 30 s without a readable file: one `Unreadable` event, then quiet until the file changes again.
//! WTE's own writes are recognised by content, not by timing (review A26): a settled file whose CRC-32 is in
//! `own_writes` or equals the link's synced CRC only updates the "last seen" stamp. A registration is used up when the
//! watcher sees that content settle (stickers P9b; check-P9a open item 1): an editor that later saves bytes equal to
//! something WTE once wrote (an undo back to the sent version, the same edit on a second link) is reported. Parsing and CRC happen here; the
//! app only drains the events. Every repaint request goes through one function that also counts it
//! (the counter `LinkUi::repaints` passed to [`LinkWatcher::start`]): with nothing changing it does not move.
//!
//! An alias (5.3 "At Save As"): the old path of a link that may still be open in the editor from before a Save As. A
//! settled change there is first copied over the link's file (atomically) and then reported as an ordinary change.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wte_core::StickerId;
use wte_core::link::FileStamp;
use wte_core::psd::{PsdDoc, PsdError, PsdLimits};

/// At most this many CRCs of WTE's own writes are remembered (the oldest is dropped).
pub const OWN_WRITES_MAX: usize = 64;

/// CRC-32 of the contents WTE wrote (shared with the link file writes, which register before the rename).
#[derive(Clone, Default)]
pub struct OwnWrites(Arc<Mutex<VecDeque<u32>>>);

impl OwnWrites {
    /// Remembers `crc` (the oldest of more than `OWN_WRITES_MAX` is dropped).
    pub fn add(&self, crc: u32) {
        if let Ok(mut q) = self.0.lock() {
            q.push_back(crc);
            while q.len() > OWN_WRITES_MAX {
                q.pop_front();
            }
        }
    }

    /// Forgets one registration of `crc` (a write that failed).
    pub fn remove(&self, crc: u32) {
        if let Ok(mut q) = self.0.lock()
            && let Some(i) = q.iter().rposition(|c| *c == crc)
        {
            q.remove(i);
        }
    }

    /// The tests look at the registrations (the watcher uses them up with [`OwnWrites::take`]).
    #[cfg(test)]
    pub fn contains(&self, crc: u32) -> bool {
        self.0.lock().is_ok_and(|q| q.contains(&crc))
    }

    /// Uses up one registration of `crc`: true when there was one (the watcher saw WTE's own write settle).
    pub fn take(&self, crc: u32) -> bool {
        let Ok(mut q) = self.0.lock() else { return false };
        match q.iter().position(|c| *c == crc) {
            Some(i) => {
                q.remove(i);
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().map_or(0, |q| q.len())
    }
}

/// The link file writes' hooks (`link::LinkFileIo`) that register WTE's own content with the watcher before the
/// rename and take it back when the write fails.
pub struct OwnWritesIo(pub OwnWrites);

impl wte_core::link::LinkFileIo for OwnWritesIo {
    fn before_rename(&mut self, crc32: u32) {
        self.0.add(crc32);
    }

    fn rename_failed(&mut self, crc32: u32) {
        self.0.remove(crc32);
    }
}

/// One watched link.
#[derive(Clone, Debug, PartialEq)]
pub struct WatchEntry {
    pub id: StickerId,
    pub path: PathBuf,
    pub alias: Option<PathBuf>,
    /// The content WTE last wrote or applied (`StickerLink::synced`).
    pub synced: FileStamp,
}

/// What the watcher reports.
#[derive(Debug)]
pub enum LinkEvent {
    /// The file settled with new content that reads as a PSD.
    Changed { id: StickerId, stamp: FileStamp, psd: Box<PsdDoc>, from_alias: bool },
    /// The file settled with content `psd::read` refuses as a whole (a colour mode, 32 bits, too large, damaged).
    Refused { id: StickerId, stamp: FileStamp, error: PsdError },
    /// The file did not become readable within `WatchTimes::give_up`.
    Unreadable { id: StickerId },
    /// The file is gone.
    Missing { id: StickerId },
}

impl LinkEvent {
    pub fn id(&self) -> StickerId {
        match self {
            LinkEvent::Changed { id, .. } | LinkEvent::Refused { id, .. } | LinkEvent::Unreadable { id } | LinkEvent::Missing { id } => *id,
        }
    }
}

/// The watcher's timing (tests shorten it).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WatchTimes {
    /// Between two polls when nothing settles (5.6: 1000 ms).
    pub poll: Duration,
    /// Between two polls while a change settles (250 ms).
    pub settle: Duration,
    /// A change that does not read within this time is reported once as unreadable (30 s).
    pub give_up: Duration,
}

impl Default for WatchTimes {
    fn default() -> WatchTimes {
        WatchTimes { poll: Duration::from_millis(1000), settle: Duration::from_millis(250), give_up: Duration::from_secs(30) }
    }
}

enum Msg {
    List(Vec<WatchEntry>),
    Poke,
}

/// The running watcher (dropping it stops the thread).
pub struct LinkWatcher {
    tx: Sender<Msg>,
    events: Receiver<LinkEvent>,
    alive: Arc<AtomicBool>,
}

impl LinkWatcher {
    /// Starts the thread. `repaint` asks the UI for a frame; `repaints` counts every request.
    pub fn start(own_writes: OwnWrites, repaint: Arc<dyn Fn() + Send + Sync>, repaints: Arc<AtomicU64>, times: WatchTimes) -> LinkWatcher {
        let (tx, rx) = std::sync::mpsc::channel();
        let (etx, events) = std::sync::mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let worker = Worker { entries: Vec::new(), own_writes, times, events: etx, repaint, repaints };
        let flag = Arc::clone(&alive);
        let spawned = std::thread::Builder::new().name("wte-link-watcher".to_owned()).spawn(move || {
            worker.run(rx);
            flag.store(false, Ordering::SeqCst);
        });
        if let Err(e) = spawned {
            log::error!("the link watcher could not start: {e}");
            alive.store(false, Ordering::SeqCst);
        }
        LinkWatcher { tx, events, alive }
    }

    /// Replaces the watched list.
    pub fn set_list(&self, list: Vec<WatchEntry>) {
        let _ = self.tx.send(Msg::List(list));
    }

    /// Polls at once.
    pub fn poke(&self) {
        let _ = self.tx.send(Msg::Poke);
    }

    /// The events reported since the last call.
    pub fn drain(&self) -> Vec<LinkEvent> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(e) => out.push(e),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// False once the thread has ended.
    pub fn alive(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.alive)
    }
}

/// (size, modification time in ns) as `metadata` reports them.
type Meta = (u64, Option<u64>);

/// What is known about one watched path.
#[derive(Clone, Debug, Default)]
struct PathState {
    /// The stamp last handled (reported or found to be WTE's own / unchanged).
    last_seen: Option<Meta>,
    /// A change being settled: its stamp at the last poll and when it was first seen.
    settle: Option<(Meta, Instant)>,
    missing_reported: bool,
}

struct Watched {
    e: WatchEntry,
    main: PathState,
    alias: PathState,
}

struct Worker {
    entries: Vec<Watched>,
    own_writes: OwnWrites,
    times: WatchTimes,
    events: Sender<LinkEvent>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    repaints: Arc<AtomicU64>,
}

/// What one poll of a path found.
enum Found {
    /// A settled change, read: its bytes, stamp and when it was first seen.
    Read(Vec<u8>, Meta, Instant),
    Missing,
}

fn meta_of(path: &Path) -> std::io::Result<Meta> {
    let m = std::fs::metadata(path)?;
    Ok((m.len(), m.modified().ok().and_then(wte_core::link::unix_ns)))
}

/// One poll of `path`: `None` while nothing is to be reported or a change is still settling.
fn poll_path(path: &Path, st: &mut PathState, synced: &FileStamp) -> Option<Found> {
    let meta = match meta_of(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            st.settle = None;
            st.last_seen = None;
            if st.missing_reported {
                return None;
            }
            st.missing_reported = true;
            return Some(Found::Missing);
        }
        Err(_) => return None,
        Ok(m) => m,
    };
    st.missing_reported = false;
    if st.settle.is_none() && st.last_seen == Some(meta) {
        return None;
    }
    if st.settle.is_none() && st.last_seen.is_none() && meta == (synced.size, synced.modified_ns) {
        st.last_seen = Some(meta);
        return None;
    }
    match st.settle {
        Some((m, since)) if m == meta => match std::fs::read(path) {
            Ok(bytes) if bytes.len() as u64 == meta.0 => Some(Found::Read(bytes, meta, since)),
            // Changed under the read, or not readable now (an editor holding it): keep settling.
            _ => {
                st.settle = Some((meta, since));
                None
            }
        },
        Some((_, since)) => {
            st.settle = Some((meta, since));
            None
        }
        None => {
            st.settle = Some((meta, Instant::now()));
            None
        }
    }
}

/// The outcome of a settled read.
enum Judged {
    /// WTE's own content, or the synced one: nothing to report.
    Own,
    Event(Result<Box<PsdDoc>, PsdError>, FileStamp),
    /// Still half written.
    Wait,
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            let settling = self.entries.iter().any(|w| w.main.settle.is_some() || w.alias.settle.is_some());
            let timeout = if settling { self.times.settle } else { self.times.poll };
            match rx.recv_timeout(timeout) {
                Ok(Msg::List(list)) => self.set_list(list),
                Ok(Msg::Poke) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let sent = self.poll_all();
            if sent > 0 {
                self.repaints.fetch_add(1, Ordering::SeqCst);
                (self.repaint)();
            }
        }
    }

    /// Takes a new list, keeping what is known about the paths that stay.
    fn set_list(&mut self, list: Vec<WatchEntry>) {
        let mut old = std::mem::take(&mut self.entries);
        for e in list {
            let kept = old.iter().position(|w| w.e.id == e.id && w.e.path == e.path).map(|i| old.swap_remove(i));
            let (main, alias) = match kept {
                Some(w) if w.e.alias == e.alias => (w.main, w.alias),
                Some(w) => (w.main, PathState::default()),
                None => (PathState::default(), PathState::default()),
            };
            self.entries.push(Watched { e, main, alias });
        }
    }

    /// Polls every path; returns the number of events sent.
    fn poll_all(&mut self) -> usize {
        let mut sent = 0;
        for i in 0..self.entries.len() {
            sent += self.poll_entry(i);
        }
        sent
    }

    fn judge(&self, bytes: &[u8], meta: Meta, since: Instant, synced: &FileStamp) -> Judged {
        let stamp = FileStamp { size: meta.0, modified_ns: meta.1, crc32: crc(bytes) };
        // WTE's own write seen settled: its registration is used up (also when the list already has it as synced).
        let own = self.own_writes.take(stamp.crc32);
        if own || stamp.same_content(synced) {
            return Judged::Own;
        }
        match wte_core::psd::read(bytes, &PsdLimits::default()) {
            Ok(doc) => Judged::Event(Ok(Box::new(doc)), stamp),
            Err(PsdError::Truncated) if since.elapsed() < self.times.give_up => Judged::Wait,
            Err(e) => Judged::Event(Err(e), stamp),
        }
    }

    fn send(&self, e: LinkEvent) -> usize {
        usize::from(self.events.send(e).is_ok())
    }

    fn poll_entry(&mut self, i: usize) -> usize {
        let mut sent = 0;
        let (id, path, synced) = {
            let w = &self.entries[i];
            (w.e.id, w.e.path.clone(), w.e.synced)
        };
        let found = poll_path(&path, &mut self.entries[i].main, &synced);
        match found {
            Some(Found::Missing) => sent += self.send(LinkEvent::Missing { id }),
            Some(Found::Read(bytes, meta, since)) => match self.judge(&bytes, meta, since, &synced) {
                Judged::Own => self.settled(i, false, meta),
                Judged::Wait => self.entries[i].main.settle = Some((meta, since)),
                Judged::Event(Ok(psd), stamp) => {
                    self.settled(i, false, meta);
                    sent += self.send(LinkEvent::Changed { id, stamp, psd, from_alias: false });
                }
                Judged::Event(Err(PsdError::Truncated), _) => {
                    self.settled(i, false, meta);
                    sent += self.send(LinkEvent::Unreadable { id });
                }
                Judged::Event(Err(error), stamp) => {
                    self.settled(i, false, meta);
                    sent += self.send(LinkEvent::Refused { id, stamp, error });
                }
            },
            None => {}
        }
        let Some(alias) = self.entries[i].e.alias.clone() else { return sent };
        // The alias is judged against the same synced content (it was a copy of it at the Save As).
        let found = poll_path(&alias, &mut self.entries[i].alias, &synced);
        if let Some(Found::Read(bytes, meta, since)) = found {
            match self.judge(&bytes, meta, since, &synced) {
                Judged::Own => self.settled(i, true, meta),
                Judged::Wait => self.entries[i].alias.settle = Some((meta, since)),
                Judged::Event(Ok(psd), _) => {
                    self.settled(i, true, meta);
                    match copy_over(&bytes, &path) {
                        Ok(main) => {
                            self.entries[i].main.last_seen = Some(main);
                            self.entries[i].main.settle = None;
                            let stamp = FileStamp { size: main.0, modified_ns: main.1, crc32: crc(&bytes) };
                            sent += self.send(LinkEvent::Changed { id, stamp, psd, from_alias: true });
                        }
                        Err(e) => log::warn!("{} could not be copied over {}: {e}", alias.display(), path.display()),
                    }
                }
                Judged::Event(Err(_), _) => self.settled(i, true, meta),
            }
        }
        sent
    }

    fn settled(&mut self, i: usize, alias: bool, meta: Meta) {
        let st = if alias { &mut self.entries[i].alias } else { &mut self.entries[i].main };
        st.last_seen = Some(meta);
        st.settle = None;
    }
}

/// Writes `bytes` over `target` through a temp file and a rename; the new file's stamp.
fn copy_over(bytes: &[u8], target: &Path) -> std::io::Result<Meta> {
    let name = target.file_name().map_or_else(|| "link.psd".into(), |n| n.to_string_lossy().into_owned());
    let tmp = target.with_file_name(format!("{name}.wte-alias-tmp"));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    meta_of(target)
}

/// CRC-32 of `bytes` (the core's, as `FileStamp` computes it).
pub fn crc(bytes: &[u8]) -> u32 {
    FileStamp::of_bytes(bytes, None).crc32
}
