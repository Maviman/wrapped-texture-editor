//! Progress reporting and cancellation for long core operations (ARCHITECTURE.md 5.4).

/// Sink for progress reports of one long operation; shared with a worker thread.
pub trait Progress: Send + Sync {
    /// `fraction` is in `0..=1` and monotone within one operation; `stage` is a short human-readable label.
    fn report(&self, fraction: f32, stage: &str);
    /// True when the operation should stop as soon as possible.
    fn cancelled(&self) -> bool;
}

/// A `Progress` that ignores reports and is never cancelled.
pub struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _fraction: f32, _stage: &str) {}

    fn cancelled(&self) -> bool {
        false
    }
}
