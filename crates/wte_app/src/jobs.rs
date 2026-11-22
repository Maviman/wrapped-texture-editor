// Owner: app-shell (A1). Background jobs of ARCHITECTURE.md 8.9: one `std::thread` worker per job, progress and
// cancellation through `JobProgress`, results delivered over an mpsc channel and polled by `App::logic`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use eframe::egui;
use wte_core::{
    CloseMode, Document, ExportReport, FontCatalog, ImportedModel, PixelBuf, StampBakeOutput, TipError, TipField, UvStats,
};

use crate::state::{BusyInfo, Pending};

/// Progress and cancellation of one job, shared between the worker and the UI thread
/// (`AtomicU32` fraction bits, `Mutex<String>` stage, `AtomicBool` cancel, `egui::Context`).
pub struct JobProgress {
    fraction_bits: AtomicU32,
    stage: Mutex<String>,
    cancel: AtomicBool,
    ctx: egui::Context,
}

impl JobProgress {
    fn new(ctx: &egui::Context, stage: &str) -> JobProgress {
        JobProgress {
            fraction_bits: AtomicU32::new(0f32.to_bits()),
            stage: Mutex::new(stage.to_owned()),
            cancel: AtomicBool::new(false),
            ctx: ctx.clone(),
        }
    }

    /// The last reported fraction, `0..=1`.
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.fraction_bits.load(Ordering::Relaxed))
    }

    /// The last reported stage label.
    pub fn stage(&self) -> String {
        self.stage.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Asks the worker to stop (seen through `Progress::cancelled`).
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl wte_core::Progress for JobProgress {
    /// Stores the report and calls `ctx.request_repaint()`.
    fn report(&self, fraction: f32, stage: &str) {
        let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.fraction_bits.store(f.to_bits(), Ordering::Relaxed);
        if let Ok(mut s) = self.stage.lock()
            && *s != stage
        {
            s.clear();
            s.push_str(stage);
        }
        self.ctx.request_repaint();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// The result a finished job hands back to the UI thread.
pub enum JobOutput {
    /// `uv_thumbs` / `uv_stats`: one entry per set (`uv_layout_image(.., 256)` and
    /// `TexelMap::build(.., 256, 256, 0).stats`), empty on `Err`.
    Imported { path: PathBuf, result: Result<ImportedModel, String>, uv_thumbs: Vec<PixelBuf>, uv_stats: Vec<UvStats> },
    Built { result: Result<Document, String>, path: Option<PathBuf>, warnings: Vec<String>, force_save_as: bool },
    /// `result` = (path, `DocumentSnapshot::history_state` of the saved snapshot, images to reuse); `scene_generation` = `Session::scene_generation` at the moment the save started.
    Saved { result: Result<(PathBuf, u64, Option<wte_core::project::PngReuse>), String>, scene_generation: u64, then: Option<Pending> },
    Exported { result: Result<ExportReport, String>, dir: PathBuf },
    Extracted { result: Result<Vec<PathBuf>, String>, dir: PathBuf },
    Fonts(FontCatalog),
    /// A brush shape baked from a model file (FEATURE_BRUSH 4.4; round 4). `request` = `PendingShape::request` of the
    /// load; a result whose number is not the newest is dropped. `close` = how the openings were to be closed in this
    /// bake; `Err(TipError::HasOpenings { .. })` makes the app ask (plan section 7 item 5).
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    BrushShape { request: u64, path: PathBuf, close: CloseMode, result: Result<Arc<TipField>, TipError> },
    /// The font picker's samples (FEATURE_TEXT 5.13; round 4), one entry per family of the catalog the job was started
    /// with, in catalog order.
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    FontSamples(Vec<FontSample>),
    /// Round 5 (R5-1): the Full bake of one stamp or text (`stamp_bakes`); `generation` = `Session::scene_generation`
    /// when the bake started. An output of another session is dropped.
    StampBake { generation: u64, output: Box<StampBakeOutput> },
}

/// The sample of one font family: "Abc 123" in its own face as a coverage bitmap (`FontCatalog::sample`).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSample {
    pub family: String,
    /// `(width, height, one coverage byte per pixel)`, at most 128 x 20 px; `None` when the face draws nothing.
    pub image: Option<(u32, u32, Vec<u8>)>,
}

/// A finished job: its output and whether the user had asked it to stop (`Jobs::cancel_modal`). A job that was asked
/// to stop and still returns `Err` was cancelled: the file flows report that as "cancelled", not as a failure.
pub struct Finished {
    pub output: JobOutput,
    pub cancel_requested: bool,
}

struct RunningJob {
    title: String,
    modal: bool,
    progress: Arc<JobProgress>,
    rx: mpsc::Receiver<JobOutput>,
}

/// The running background jobs: one `std::thread` worker per job
/// (`Vec<(title, modal, Arc<JobProgress>, mpsc::Receiver<JobOutput>)>`).
pub struct Jobs {
    running: Vec<RunningJob>,
}

impl Jobs {
    /// No running jobs.
    pub fn new() -> Jobs {
        Jobs { running: Vec::new() }
    }

    /// Starts `f` on a worker thread. `modal` jobs block the UI behind the progress dialog.
    pub fn spawn(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        modal: bool,
        f: impl FnOnce(&JobProgress) -> JobOutput + Send + 'static,
    ) {
        self.spawn_staged(ctx, title, modal, "Starting", f);
    }

    /// `spawn` with the stage shown until the job reports one (round 5: a stamp bake shows its title alone, `""`).
    /// False when no worker thread could be started (logged; `f` is dropped).
    pub fn spawn_staged(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        modal: bool,
        stage: &str,
        f: impl FnOnce(&JobProgress) -> JobOutput + Send + 'static,
    ) -> bool {
        let progress = Arc::new(JobProgress::new(ctx, stage));
        let (tx, rx) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let thread_name = format!("job: {title}");
        let log_title = title.to_owned();
        let spawned = std::thread::Builder::new().name(thread_name).spawn(move || {
            let started = std::time::Instant::now();
            let out = f(&worker_progress);
            log::info!("job \"{log_title}\" finished in {:.2} s", started.elapsed().as_secs_f64());
            // The receiver is gone only when the app is shutting down.
            let _ = tx.send(out);
            worker_progress.ctx.request_repaint();
        });
        match spawned {
            Ok(_) => {
                log::info!("job \"{title}\" started (modal: {modal})");
                self.running.push(RunningJob { title: title.to_owned(), modal, progress, rx });
                true
            }
            Err(e) => {
                log::error!("cannot start a worker thread for \"{title}\": {e}");
                false
            }
        }
    }

    /// Round 5: removes the (first) running job with this title and blocks until it delivers its output. Used only by
    /// `stamp_bakes::settle`, the one place the UI thread waits for a worker (4.6). `None` when no such job runs or
    /// its worker ended without a result.
    pub fn wait_for(&mut self, title: &str) -> Option<JobOutput> {
        let index = self.running.iter().position(|j| j.title == title)?;
        let job = self.running.remove(index);
        match job.rx.recv() {
            Ok(output) => Some(output),
            Err(_) => {
                log::error!("job \"{}\" ended without a result", job.title);
                None
            }
        }
    }

    /// The outputs of the jobs that finished since the last call.
    pub fn poll(&mut self) -> Vec<Finished> {
        let mut out = Vec::new();
        self.running.retain(|job| match job.rx.try_recv() {
            Ok(output) => {
                out.push(Finished { output, cancel_requested: wte_core::Progress::cancelled(&*job.progress) });
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                // Only possible if the worker died without sending (a panic exits the process first, see main.rs).
                log::error!("job \"{}\" ended without a result", job.title);
                false
            }
        });
        out
    }

    /// The first modal job, else the first job.
    pub fn busy(&self) -> Option<BusyInfo> {
        let job = self.running.iter().find(|j| j.modal).or_else(|| self.running.first())?;
        Some(BusyInfo {
            title: job.title.clone(),
            stage: job.progress.stage(),
            fraction: job.progress.fraction(),
            modal: job.modal,
        })
    }

    /// Asks the modal job to stop.
    pub fn cancel_modal(&self) {
        for job in self.running.iter().filter(|j| j.modal) {
            log::info!("cancel requested for job \"{}\"", job.title);
            job.progress.request_cancel();
        }
    }

    /// True while any job runs.
    pub fn any_running(&self) -> bool {
        !self.running.is_empty()
    }

    /// True while a modal job runs.
    pub fn modal_running(&self) -> bool {
        self.running.iter().any(|j| j.modal)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wte_core::Progress;

    use super::*;

    fn wait_for(jobs: &mut Jobs) -> Vec<JobOutput> {
        wait_for_finished(jobs).into_iter().map(|f| f.output).collect()
    }

    fn wait_for_finished(jobs: &mut Jobs) -> Vec<Finished> {
        let start = Instant::now();
        loop {
            let out = jobs.poll();
            if !out.is_empty() || start.elapsed() > Duration::from_secs(20) {
                return out;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_reports_progress_and_delivers_its_output() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        assert!(jobs.busy().is_none() && !jobs.any_running() && !jobs.modal_running());
        let (go_tx, go_rx) = mpsc::channel::<()>();
        jobs.spawn(&ctx, "Exporting", true, move |p| {
            p.report(0.5, "Writing");
            go_rx.recv().ok();
            JobOutput::Extracted { result: Ok(vec![PathBuf::from("a.png")]), dir: PathBuf::from("out") }
        });
        assert!(jobs.any_running() && jobs.modal_running());
        let start = Instant::now();
        while jobs.busy().map(|b| b.stage) != Some("Writing".to_owned()) {
            assert!(start.elapsed() < Duration::from_secs(20), "no progress report");
            std::thread::sleep(Duration::from_millis(2));
        }
        let busy = jobs.busy().unwrap();
        assert_eq!(busy, BusyInfo { title: "Exporting".into(), stage: "Writing".into(), fraction: 0.5, modal: true });
        go_tx.send(()).unwrap();
        let out = wait_for(&mut jobs);
        assert!(matches!(&out[..], [JobOutput::Extracted { result: Ok(files), .. }] if files.len() == 1));
        assert!(!jobs.any_running() && jobs.busy().is_none());
    }

    #[test]
    fn busy_prefers_the_modal_job_and_cancel_reaches_it() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        jobs.spawn(&ctx, "Fonts", false, move |_| {
            stop_rx.recv().ok();
            JobOutput::Fonts(FontCatalog::bundled())
        });
        jobs.spawn(&ctx, "Saving", true, |p| {
            let start = Instant::now();
            while !p.cancelled() && start.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(2));
            }
            JobOutput::Saved { result: Err(if p.cancelled() { "cancelled" } else { "timeout" }.into()), scene_generation: 1, then: None }
        });
        assert_eq!(jobs.busy().unwrap().title, "Saving");
        jobs.cancel_modal();
        let out = wait_for_finished(&mut jobs);
        assert!(matches!(&out[..], [Finished { output: JobOutput::Saved { result: Err(e), .. }, cancel_requested: true }] if e == "cancelled"));
        assert!(!jobs.modal_running() && jobs.any_running());
        assert_eq!(jobs.busy().unwrap().title, "Fonts");
        assert!(!jobs.busy().unwrap().modal);
        stop_tx.send(()).unwrap();
        assert!(matches!(&wait_for(&mut jobs)[..], [JobOutput::Fonts(_)]));
    }

    #[test]
    fn wait_for_blocks_on_the_named_job_and_removes_it() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        jobs.spawn(&ctx, "Fonts", false, move |_| {
            stop_rx.recv().ok();
            JobOutput::Fonts(FontCatalog::bundled())
        });
        assert!(jobs.spawn_staged(&ctx, "Finishing stamp", false, "", |_| {
            std::thread::sleep(Duration::from_millis(30));
            JobOutput::Extracted { result: Ok(Vec::new()), dir: PathBuf::from("x") }
        }));
        assert!(jobs.wait_for("No such job").is_none());
        assert!(matches!(jobs.wait_for("Finishing stamp"), Some(JobOutput::Extracted { .. })));
        assert_eq!(jobs.busy().unwrap().title, "Fonts", "only the named job was removed");
        assert!(jobs.wait_for("Finishing stamp").is_none());
        stop_tx.send(()).unwrap();
        assert!(matches!(&wait_for(&mut jobs)[..], [JobOutput::Fonts(_)]));
    }

    #[test]
    fn progress_values_are_sanitized() {
        let ctx = egui::Context::default();
        let p = JobProgress::new(&ctx, "x");
        p.report(f32::NAN, "a");
        assert_eq!(p.fraction(), 0.0);
        p.report(7.0, "b");
        assert_eq!(p.fraction(), 1.0);
        assert_eq!(p.stage(), "b");
        assert!(!p.cancelled());
        p.request_cancel();
        assert!(p.cancelled());
    }
}
