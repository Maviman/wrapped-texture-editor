// Owner: app-shell (A1). Self-test runner of ARCHITECTURE.md 8.11: deterministic input injection, scenario driving,
// screenshot capture and the JSON report. The scenario table lives in `scenarios.rs` (app-selftest).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::{egui, wgpu};

use crate::action::AppAction;
use crate::cli::CliArgs;
use crate::gpu::Gpu;
use crate::pen::{PenKind, PenRecord};
use crate::state::{AppState, ToolKind, ViewId};

pub mod scenarios;

/// Exit code of a failed `Setup` / `Assert` / `Dyn`, an error toast or an unknown scenario.
pub const EXIT_FAILED: i32 = 2;
/// Exit code of a timeout.
pub const EXIT_TIMEOUT: i32 = 3;
/// A single step may take this long (wall clock); scenarios may raise it (`scenarios::timeouts`), the environment
/// variable `WTE_SELFTEST_STEP_TIMEOUT` (seconds) overrides both.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// The whole run may take this long (wall clock); scenarios may raise it, `WTE_SELFTEST_RUN_TIMEOUT` (seconds)
/// overrides both.
const RUN_TIMEOUT: Duration = Duration::from_secs(180);
/// A `Resize` step waits this many frames for the new size.
const RESIZE_FRAMES: u32 = 10;
/// Scenario steps start at the latest after this many frames, even if the window size does not match.
const SIZE_FRAMES: u64 = 120;
/// At most one `InnerSize` command every this many frames.
const RESIZE_INTERVAL: u64 = 10;
/// `WaitIdle` needs this many consecutive idle frames.
const IDLE_FRAMES: u32 = 3;
/// After `Exit`, the process is ended directly if the window has not closed within this many frames.
const EXIT_GRACE_FRAMES: u64 = 600;

/// Values a scenario carries from one step to a later one (hashes, counts), plus the output location.
pub struct Scratch {
    pub values: BTreeMap<&'static str, u64>,
    /// OUT without ".png"; scenario files go to `<out_stem>_assets`, `<out_stem>_export`, `<out_stem>.wtep`.
    pub out_stem: PathBuf,
}

/// Stickers P6a (STICKER_PLAN 3.1, 8.2): where a pointer, click, wheel, hover or pen step goes: a rendered view, or the
/// sticker canvas (which is not a `ViewId`, review A17; its rect is `AppState::sticker_canvas_rect`).
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Target {
    View(ViewId),
    StickerCanvas,
}

impl From<ViewId> for Target {
    fn from(view: ViewId) -> Target {
        Target::View(view)
    }
}

impl std::fmt::Debug for Target {
    /// A view prints as before (`Main3D`), so step descriptions in the logs keep their wording.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::View(view) => write!(f, "{view:?}"),
            Target::StickerCanvas => write!(f, "StickerCanvas"),
        }
    }
}

/// One step of a scenario.
pub enum Step {
    Do(AppAction),
    /// `path`: normalized 0..1 in the target's rect.
    Pointer { view: Target, path: Vec<[f32; 2]>, button: egui::PointerButton, mods: egui::Modifiers, points_per_frame: usize },
    Click { view: Target, at: [f32; 2], button: egui::PointerButton, mods: egui::Modifiers },
    /// Primary-button press at the first point, moves through the others, release at the last one; positions in
    /// screen points (for panel controls, e.g. the Layers rows of `AppState::panel_rects`). One point = a click.
    ScreenPointer { path: Vec<egui::Pos2> },
    /// One click at a screen position (points) with any button and modifiers: Ctrl / Shift-click and the right-click
    /// on a Layers row, a click on an item of the row's context menu.
    ScreenClick { at: egui::Pos2, button: egui::PointerButton, mods: egui::Modifiers },
    Scroll { view: Target, at: [f32; 2], notches: f32, mods: egui::Modifiers },
    /// Stickers P6b: wheel notches at a screen position (points): scrolls a page of the side panel to a control a
    /// scenario clicks next (`ScreenClick`).
    ScreenScroll { at: egui::Pos2, notches: f32 },
    /// The pointer moves to `at` (normalized 0..1 in the view rect) without a button: one hover frame (round 4, for the
    /// Text tool's picture under the pointer and the brush cursor).
    #[allow(dead_code, reason = "round-4 contract: used by the wave-2 packages")]
    Hover { view: Target, at: [f32; 2] },
    Key(egui::Modifiers, egui::Key),
    /// Branch `pen-input` (ARCHITECTURE 8.11): a recorded Windows Ink pen fed in-process: per sample exactly the events
    /// egui-winit makes for the pen (`Touch` with its force, the emulated `PointerMoved` / `PointerButton` /
    /// `PointerGone`) plus the window procedure's `PenRecord`s (`pen_frames`); never OS input. `samples.at` is
    /// normalized in the view rect (values outside 0..1 reach the panels); pressure 0 = hover. The first contact sample
    /// is the Down, the last one is followed by an Up, a Leave record ends the step. `inverted` = the eraser end,
    /// `barrel` = the barrel button held.
    Pen { view: Target, samples: Vec<PenPoint>, inverted: bool, barrel: bool, points_per_frame: usize },
    /// Window inner size in physical pixels (one `ViewportCommand::InnerSize`), then `RESIZE_FRAMES` frames of waiting.
    Resize([u32; 2]),
    WaitFrames(u32),
    /// Stickers P6a (review A14): the runner keeps ticking until `n` milliseconds of wall-clock time have passed (the
    /// runner repaints every frame, so "nothing happened for 3 s" cannot be told from a frame count). What follows
    /// asserts state, never a frame counter.
    #[allow(dead_code, reason = "stickers P9a: the link watcher's idle check")]
    WaitMs(u64),
    WaitIdle,
    /// Direct state preparation or bookkeeping: register the sample stamp image as a blob, write asset files, remember
    /// a hash in `Scratch`.
    Setup(&'static str, fn(&mut AppState, &mut Scratch) -> Result<(), String>),
    Assert(&'static str, fn(&AppState, &Scratch) -> Result<(), String>),
    /// Steps computed at run time from the live state and spliced in at this position — for pointer paths that depend
    /// on camera framing and layout (dragging a gizmo handle: `ViewXform::for_view` + `view::gizmo::gizmo_shape`,
    /// converted to normalized view coordinates).
    Dyn(&'static str, fn(&AppState, &Scratch) -> Result<Vec<Step>, String>),
    /// Branch `companion`: waits (one check per frame, at most the step timeout) until the condition is `Ok(true)`;
    /// `Err` fails the scenario. For events from outside the frame loop (a test client on another thread).
    Until(&'static str, fn(&AppState, &Scratch) -> Result<bool, String>),
    Screenshot,
    /// Round 4 (gate): a named capture in the middle of a run, written to `<out_stem>_<name>.png` (the run's own
    /// `OUT.png` is still the final `Screenshot`). Lets a scenario keep the picture of an intermediate frame (the
    /// hover of `text_tab` and `brush_shape`). Stickers P6a (review B15): `expect` says what the picture must show;
    /// the runner writes every shot's name and `expect` into `OUT.json` (`"shots"`) for the checker and the gate. The
    /// sticker scenarios must give one (the scenario-count test); older scenarios have "".
    Shot { name: &'static str, expect: &'static str },
    /// Round 5 (R5-1): switches the runner's wait for the Final stamp bakes after input and `Do` steps off (`false`)
    /// and on again (`true`, the default). With it off the next step starts while a bake still runs (the scenario
    /// `final_bake_async`); `WaitIdle` always waits for the bakes.
    BakeWait(bool),
    Exit,
}

impl Step {
    /// Short description for the log and error messages.
    fn describe(&self) -> String {
        match self {
            Step::Do(a) => {
                let text = format!("{a:?}");
                let short: String = text.chars().take(120).collect();
                format!("Do({short})")
            }
            Step::Pointer { view, path, button, .. } => format!("Pointer({view:?}, {} points, {button:?})", path.len()),
            Step::Click { view, at, button, .. } => format!("Click({view:?}, {at:?}, {button:?})"),
            Step::ScreenPointer { path } => format!("ScreenPointer({} points from {:?})", path.len(), path.first()),
            Step::ScreenClick { at, button, mods } => format!("ScreenClick({at:?}, {button:?}, {mods:?})"),
            Step::Scroll { view, at, notches, .. } => format!("Scroll({view:?}, {at:?}, {notches})"),
            Step::ScreenScroll { at, notches } => format!("ScreenScroll({at:?}, {notches})"),
            Step::Hover { view, at } => format!("Hover({view:?}, {at:?})"),
            Step::Key(mods, key) => format!("Key({mods:?}, {key:?})"),
            Step::Pen { view, samples, inverted, barrel, .. } => {
                let contact = samples.iter().filter(|s| s.pressure > 0.0).count();
                format!("Pen({view:?}, {} samples, {contact} in contact, inverted {inverted}, barrel {barrel})", samples.len())
            }
            Step::Resize([w, h]) => format!("Resize({w}x{h})"),
            Step::WaitFrames(n) => format!("WaitFrames({n})"),
            Step::WaitMs(n) => format!("WaitMs({n})"),
            Step::WaitIdle => "WaitIdle".to_owned(),
            Step::Setup(name, _) => format!("Setup({name})"),
            Step::Assert(name, _) => format!("Assert({name})"),
            Step::Dyn(name, _) => format!("Dyn({name})"),
            Step::Until(name, _) => format!("Until({name})"),
            Step::Screenshot => "Screenshot".to_owned(),
            Step::Shot { name, .. } => format!("Shot({name})"),
            Step::BakeWait(on) => format!("BakeWait({on})"),
            Step::Exit => "Exit".to_owned(),
        }
    }
}

/// One sample of `Step::Pen`: normalized view position and pressure (0 = hovering).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PenPoint {
    pub at: [f32; 2],
    pub pressure: f32,
}

/// The events injected in one frame, the modifier state of that frame and the pen records of the frame (branch
/// `pen-input`: what the window procedure would have queued).
#[derive(Clone, Debug, PartialEq)]
struct FrameInput {
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    pen: Vec<PenRecord>,
}

/// A step that spans several frames.
enum Active {
    /// Injected input is pending (done when the queue is drained and the last frame has been processed).
    Input,
    Frames(u32),
    Idle { count: u32 },
    /// A screenshot is on its way; `Some(name)` = a named mid-run capture (`Step::Shot`).
    Capture(mpsc::Receiver<Arc<egui::ColorImage>>, Option<&'static str>),
    /// Round 5: after a `Do` step, until no Final stamp bake is pending (`AppState::final_bakes_pending`).
    Bakes,
    /// Branch `companion`: `Step::Until`.
    Until(&'static str, fn(&AppState, &Scratch) -> Result<bool, String>),
    /// Stickers P6a: `Step::WaitMs`, until this wall-clock instant.
    WaitUntil(Instant),
}

/// Drives a scenario: injects input, waits, asserts, captures the screenshot and writes the report.
pub struct Runner {
    scenario: String,
    steps: VecDeque<Step>,
    active: Option<(Active, String)>,
    frame: u64,
    scratch: Scratch,
    inject: VecDeque<FrameInput>,
    last_inject_frame: Option<u64>,
    /// The modifier state last announced to egui.
    modifiers: egui::Modifiers,
    out_png: PathBuf,
    out_json: PathBuf,
    requested_size: [u32; 2],
    size_clamped: bool,
    started: bool,
    last_resize_frame: Option<u64>,
    errors: Vec<String>,
    code: i32,
    failing: bool,
    exit_code: Arc<AtomicI32>,
    run_start: Instant,
    step_start: Instant,
    step_timeout: Duration,
    run_timeout: Duration,
    error_baseline: usize,
    captured_size: Option<[u32; 2]>,
    last_rects: HashMap<ViewId, egui::Rect>,
    exited_at: Option<u64>,
    /// Round 5: input and `Do` steps also wait for the pending Final stamp bakes (`Step::BakeWait`; default on).
    bake_wait: bool,
    /// Stickers P6a (review B15): the named shots taken, with what each must show (`"shots"` of `OUT.json`).
    shots: Vec<(&'static str, &'static str)>,
}

impl Runner {
    /// `None` without `--screenshot`; an unknown scenario gives an error scenario.
    pub fn new(args: &CliArgs, exit_code: Arc<AtomicI32>) -> Option<Runner> {
        let out_png = args.screenshot.clone()?;
        let out_stem = args.out_stem()?;
        let out_json = with_suffix(&out_stem, ".json");
        let mut errors = Vec::new();
        let mut code = 0;
        let steps: VecDeque<Step> = match scenarios::scenario(&args.scenario, args) {
            Some(steps) => steps.into(),
            None => {
                errors.push(format!(
                    "unknown scenario \"{}\" (known: {})",
                    args.scenario,
                    scenarios::scenario_names().join(", ")
                ));
                code = EXIT_FAILED;
                exit_code.store(code, Ordering::SeqCst);
                vec![Step::Screenshot, Step::Exit].into()
            }
        };
        let (step_timeout, run_timeout) = timeouts(&args.scenario);
        log::info!(
            "self-test scenario \"{}\": {} steps, output {}, timeouts {} s per step / {} s per run",
            args.scenario,
            steps.len(),
            out_png.display(),
            step_timeout.as_secs(),
            run_timeout.as_secs()
        );
        let now = Instant::now();
        Some(Runner {
            scenario: args.scenario.clone(),
            failing: code != 0,
            steps,
            active: None,
            frame: 0,
            scratch: Scratch { values: BTreeMap::new(), out_stem },
            inject: VecDeque::new(),
            last_inject_frame: None,
            modifiers: egui::Modifiers::NONE,
            out_png,
            out_json,
            requested_size: args.size,
            size_clamped: false,
            started: false,
            last_resize_frame: None,
            errors,
            code,
            exit_code,
            run_start: now,
            step_start: now,
            step_timeout,
            run_timeout,
            error_baseline: 0,
            captured_size: None,
            last_rects: HashMap::new(),
            exited_at: None,
            bake_wait: true,
            shots: Vec::new(),
        })
    }

    /// Drops the real pointer / key / wheel events and appends the injected ones (determinism rules of 8.11); returns
    /// the injected pen records of the frame (branch `pen-input`), which the app hands to the pen router.
    pub fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) -> Vec<PenRecord> {
        raw.events.retain(|e| {
            !matches!(
                e,
                egui::Event::PointerMoved(_)
                    | egui::Event::PointerButton { .. }
                    | egui::Event::MouseMoved(_)
                    | egui::Event::PointerGone
                    | egui::Event::MouseWheel { .. }
                    | egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::Zoom(_)
                    | egui::Event::Rotate(_)
                    | egui::Event::WindowFocused(_)
                    | egui::Event::ModifiersChanged(_)
                    | egui::Event::Touch { .. }
                    | egui::Event::Ime(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
                    | egui::Event::Paste(_)
                    | egui::Event::PasteImage(_)
                    | egui::Event::MiddleClickPaste { .. }
            )
        });
        raw.hovered_files.clear();
        raw.dropped_files.clear();
        // The window opens in the background (`main`), so the OS never gives it the focus; a run behaves as if it had
        // it (text cursors, `Response::has_focus`), whatever the user clicks meanwhile.
        raw.focused = true;
        // egui 0.37 has no `RawInput::modifiers`: the modifier state is carried by `Event::ModifiersChanged`, so the
        // injected state is announced whenever it differs from the last one (NONE in frames without injected input).
        let input = self.inject.pop_front();
        let modifiers = input.as_ref().map_or(egui::Modifiers::NONE, |i| i.modifiers);
        if modifiers != self.modifiers {
            raw.events.push(egui::Event::ModifiersChanged(modifiers));
            self.modifiers = modifiers;
        }
        raw.time = Some(self.frame as f64 / 60.0);
        raw.predicted_dt = 1.0 / 60.0;
        let Some(input) = input else { return Vec::new() };
        raw.events.extend(input.events);
        self.last_inject_frame = Some(self.frame);
        input.pen
    }

    /// Branch `pen-input`: what a pen in contact did this frame, for the `pen` scenario's asserts (scratch values
    /// `pen_contact_frames`, `pen_eraser_frames`, `pen_readout_ok`).
    fn probe_pen(&mut self, state: &AppState) {
        let f = &state.pen.frame;
        if !f.in_contact {
            return;
        }
        *self.scratch.values.entry("pen_contact_frames").or_insert(0) += 1;
        // Stickers P7a: in the Sticker view the eraser end selects the sticker Eraser.
        let sticker_eraser = crate::sticker::edit_of(state).is_some_and(|e| e.tool == crate::state::StickerTool::Eraser);
        if state.tool == ToolKind::Eraser || sticker_eraser {
            *self.scratch.values.entry("pen_eraser_frames").or_insert(0) += 1;
        }
        if f.readout().is_some_and(|t| t.starts_with("Pen ") && t.ends_with(" %")) {
            self.scratch.values.insert("pen_readout_ok", 1);
        }
    }

    /// Advances the scenario by one frame; called from `App::logic`.
    pub fn tick(&mut self, ctx: &egui::Context, state: &mut AppState, gpu: &Gpu, tools_busy: bool) {
        // Keep frames coming even while the window is minimized or hidden (the one place allowed to send a viewport
        // command every frame, 8.13).
        ctx.request_repaint();
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        let frame = self.frame;
        self.frame += 1;
        self.probe_pen(state);

        if frame == 0 {
            ctx.all_styles_mut(|s| s.animation_time = 0.0);
            ctx.set_zoom_factor(1.0 / ctx.native_pixels_per_point().unwrap_or(1.0));
            return;
        }
        if let Some(exited) = self.exited_at {
            if frame > exited + EXIT_GRACE_FRAMES {
                log::warn!("the window did not close after the self-test; exiting directly");
                log::logger().flush();
                std::process::exit(self.code);
            }
            return;
        }
        if !self.started {
            self.size_phase(ctx, frame);
            if !self.started {
                return;
            }
            self.step_start = Instant::now();
        }

        // Error toasts fail the scenario (exit code 2); the remaining steps are skipped, a capture is still attempted.
        if state.toasts.error_count() > self.error_baseline {
            let new: Vec<String> = state.toasts.error_texts()[self.error_baseline.min(state.toasts.error_texts().len())..].to_vec();
            self.error_baseline = state.toasts.error_count();
            for text in new {
                // fix_export item 9: a scenario that drives a failure on purpose announces its error toasts
                // (`expected_error_toasts` in the scratch values); each one announced is taken instead of failing.
                let expected = self.scratch.values.get("expected_error_toasts").copied().unwrap_or(0);
                if expected > 0 {
                    self.scratch.values.insert("expected_error_toasts", expected - 1);
                    log::info!("expected error toast: {text}");
                } else {
                    self.fail(EXIT_FAILED, format!("error toast: {text}"));
                }
            }
        }
        if self.run_start.elapsed() > self.run_timeout && !self.failing {
            self.fail(EXIT_TIMEOUT, format!("the run took longer than {} s", self.run_timeout.as_secs()));
        }
        if self.step_start.elapsed() > self.step_timeout
            && let Some((_, name)) = &self.active
        {
            let name = name.clone();
            self.fail(EXIT_TIMEOUT, format!("step {name} took longer than {} s", self.step_timeout.as_secs()));
        }

        if let Some((active, name)) = self.active.take() {
            match self.poll_active(active, frame, state, gpu, tools_busy) {
                Some(still) => {
                    self.active = Some((still, name));
                    self.last_rects = state.view_rects.clone();
                    return;
                }
                None => log::info!("frame {frame}: step done: {name} ({} ms)", self.step_start.elapsed().as_millis()),
            }
        }
        self.last_rects = state.view_rects.clone();
        self.start_next(ctx, state, frame);
    }

    /// Window size settling (8.11): from frame 1 on, resize until the physical size matches, at most for 120 frames.
    fn size_phase(&mut self, ctx: &egui::Context, frame: u64) {
        let ppp = ctx.pixels_per_point();
        let actual = (ctx.content_rect().size() * ppp).round();
        let [w, h] = self.requested_size;
        if actual == egui::vec2(w as f32, h as f32) {
            log::info!("frame {frame}: window size {w}x{h} reached");
            self.started = true;
            return;
        }
        if frame >= SIZE_FRAMES {
            log::warn!("frame {frame}: window size stays {}x{} instead of {w}x{h}", actual.x, actual.y);
            self.size_clamped = true;
            self.started = true;
            return;
        }
        if self.last_resize_frame.is_none_or(|f| frame >= f + RESIZE_INTERVAL) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32) / ppp));
            self.last_resize_frame = Some(frame);
        }
    }

    /// Continues a multi-frame step; `None` when it is complete.
    fn poll_active(&mut self, active: Active, frame: u64, state: &AppState, gpu: &Gpu, tools_busy: bool) -> Option<Active> {
        match active {
            Active::Input => {
                let processed = self.inject.is_empty() && self.last_inject_frame.is_none_or(|f| f < frame);
                // Round 4 (FEATURE_BRUSH 7.3): an input step also waits until the tools are idle, so a pointer step
                // ends only when its stroke is complete, however long its paint trails the pointer (stroke pacing).
                // Round 5: and until the Final stamp bakes it caused have landed (unless `BakeWait(false)`).
                (!processed || tools_busy || self.waits_for_bakes(state)).then_some(Active::Input)
            }
            Active::Bakes => self.waits_for_bakes(state).then_some(Active::Bakes),
            Active::Until(label, f) => match f(state, &self.scratch) {
                Ok(true) => None,
                Ok(false) => Some(Active::Until(label, f)),
                Err(e) => {
                    self.fail(EXIT_FAILED, format!("Until \"{label}\" failed: {e}"));
                    None
                }
            },
            Active::Frames(n) => (n > 1).then_some(Active::Frames(n - 1)),
            Active::WaitUntil(at) => wait_until(at),
            Active::Idle { count } => {
                let idle = is_idle(state, tools_busy) && state.view_rects == self.last_rects;
                let count = if idle { count + 1 } else { 0 };
                (count < IDLE_FRAMES).then_some(Active::Idle { count })
            }
            Active::Capture(rx, shot) => {
                // The readback uses map_async; nothing else polls the device (8.1, pitfall 5).
                let _ = gpu.device.poll(wgpu::PollType::Poll);
                match rx.try_recv() {
                    Ok(image) => {
                        self.write_png(&image, shot);
                        None
                    }
                    Err(mpsc::TryRecvError::Empty) => Some(Active::Capture(rx, shot)),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.fail(EXIT_FAILED, "the screenshot callback was dropped".to_owned());
                        None
                    }
                }
            }
        }
    }

    /// Starts the next step (at most one per frame).
    fn start_next(&mut self, ctx: &egui::Context, state: &mut AppState, frame: u64) {
        let step = self.steps.pop_front().unwrap_or(Step::Exit);
        let name = step.describe();
        log::info!("frame {frame}: step {name}");
        self.step_start = Instant::now();
        match step {
            Step::Do(action) => {
                state.push(action);
                // Round 5: the action is dispatched in this frame; from the next one on the step waits for the Final
                // stamp bakes it caused (no extra frame when none is pending).
                if self.bake_wait {
                    self.active = Some((Active::Bakes, name));
                }
            }
            Step::Pointer { view, path, button, mods, points_per_frame } => {
                if let Some(rect) = self.view_rect(state, view, &name) {
                    let points: Vec<egui::Pos2> = path.iter().map(|p| to_screen(rect, *p)).collect();
                    self.inject.extend(pointer_frames(&points, button, mods, points_per_frame));
                    self.active = Some((Active::Input, name));
                }
            }
            Step::Click { view, at, button, mods } => {
                if let Some(rect) = self.view_rect(state, view, &name) {
                    self.inject.extend(click_frames(to_screen(rect, at), button, mods));
                    self.active = Some((Active::Input, name));
                }
            }
            Step::ScreenPointer { path } => {
                self.inject.extend(pointer_frames(&path, egui::PointerButton::Primary, egui::Modifiers::NONE, 1));
                self.active = Some((Active::Input, name));
            }
            Step::ScreenClick { at, button, mods } => {
                self.inject.extend(click_frames(at, button, mods));
                self.active = Some((Active::Input, name));
            }
            Step::ScreenScroll { at, notches } => {
                self.inject.extend(scroll_frames(at, notches, egui::Modifiers::NONE));
                self.active = Some((Active::Input, name));
            }
            Step::Scroll { view, at, notches, mods } => {
                if let Some(rect) = self.view_rect(state, view, &name) {
                    self.inject.extend(scroll_frames(to_screen(rect, at), notches, mods));
                    self.active = Some((Active::Input, name));
                }
            }
            Step::Hover { view, at } => {
                if let Some(rect) = self.view_rect(state, view, &name) {
                    self.inject.extend(hover_frames(to_screen(rect, at)));
                    self.active = Some((Active::Input, name));
                }
            }
            Step::Key(mods, key) => {
                self.inject.extend(key_frames(key, mods));
                self.active = Some((Active::Input, name));
            }
            Step::Pen { view, samples, inverted, barrel, points_per_frame } => {
                if let Some(rect) = self.view_rect(state, view, &name) {
                    let points: Vec<(egui::Pos2, f32)> = samples.iter().map(|s| (to_screen(rect, s.at), s.pressure)).collect();
                    self.inject.extend(pen_frames(&points, inverted, barrel, points_per_frame));
                    self.active = Some((Active::Input, name));
                }
            }
            Step::Resize([w, h]) => {
                let ppp = ctx.pixels_per_point();
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32) / ppp));
                self.active = Some((Active::Frames(RESIZE_FRAMES), name));
            }
            Step::WaitFrames(n) => {
                if n > 0 {
                    self.active = Some((Active::Frames(n), name));
                }
            }
            Step::WaitMs(ms) => self.active = Some((Active::WaitUntil(Instant::now() + Duration::from_millis(ms)), name)),
            Step::WaitIdle => self.active = Some((Active::Idle { count: 0 }, name)),
            Step::Setup(label, f) => {
                if let Err(e) = f(state, &mut self.scratch) {
                    self.fail(EXIT_FAILED, format!("Setup \"{label}\" failed: {e}"));
                }
            }
            Step::Assert(label, f) => match f(state, &self.scratch) {
                Ok(()) => log::info!("assert ok: {label}"),
                Err(e) => self.fail(EXIT_FAILED, format!("Assert \"{label}\" failed: {e}")),
            },
            Step::Dyn(label, f) => match f(state, &self.scratch) {
                Ok(steps) => {
                    log::info!("Dyn \"{label}\": {} steps", steps.len());
                    for s in steps.into_iter().rev() {
                        self.steps.push_front(s);
                    }
                }
                Err(e) => self.fail(EXIT_FAILED, format!("Dyn \"{label}\" failed: {e}")),
            },
            Step::Until(label, f) => self.active = Some((Active::Until(label, f), name)),
            Step::Screenshot => self.capture(ctx, None, name),
            Step::Shot { name: shot, expect } => {
                self.shots.push((shot, expect));
                self.capture(ctx, Some(shot), name);
            }
            Step::BakeWait(on) => self.bake_wait = on,
            Step::Exit => self.finish(ctx, state, frame),
        }
    }

    /// The rect of a step's target in the previous frame (a view, or the sticker canvas); fails the scenario when it
    /// was not laid out.
    fn view_rect(&mut self, state: &AppState, view: Target, step: &str) -> Option<egui::Rect> {
        let rect = match view {
            Target::View(v) => state.view_rects.get(&v).copied(),
            Target::StickerCanvas => state.sticker_canvas_rect,
        };
        match rect {
            Some(r) if r.width() > 0.0 && r.height() > 0.0 => Some(r),
            _ => {
                self.fail(EXIT_FAILED, format!("{step}: the view {view:?} is not shown"));
                None
            }
        }
    }

    /// Records a failure; the remaining steps are skipped and a capture is still attempted.
    fn fail(&mut self, code: i32, message: String) {
        log::error!("self-test: {message}");
        self.errors.push(message);
        if self.code == 0 {
            self.code = code;
            self.exit_code.store(code, Ordering::SeqCst);
        }
        self.inject.clear();
        let capturing = matches!(self.active, Some((Active::Capture(_, None), _)));
        if self.failing {
            // A failure while already failing (e.g. the capture itself timed out): just end the run.
            self.steps = VecDeque::from([Step::Exit]);
            if capturing {
                self.active = None;
            }
            return;
        }
        self.failing = true;
        if capturing {
            self.steps = VecDeque::from([Step::Exit]);
        } else {
            self.active = None;
            self.steps = VecDeque::from([Step::Screenshot, Step::Exit]);
        }
    }

    /// True while the runner waits for pending Final stamp bakes (`BakeWait` on and a bake pending). Round 8: also for
    /// the wrapped stamps' Preview and param-only jobs (`AppState::wrap_jobs_pending`).
    fn waits_for_bakes(&self, state: &AppState) -> bool {
        self.bake_wait && (state.final_bakes_pending || state.wrap_jobs_pending)
    }

    /// Requests a capture of the next painted frame (`shot`: the name of a mid-run capture, `None` = `OUT.png`).
    fn capture(&mut self, ctx: &egui::Context, shot: Option<&'static str>, name: String) {
        let (tx, rx) = mpsc::channel();
        ctx.request_screenshot(move |image| {
            let _ = tx.send(image);
        });
        self.active = Some((Active::Capture(rx, shot), name));
    }

    /// The file of a capture: `OUT.png`, or `<out_stem>_<name>.png` for a named mid-run capture (`Step::Shot`).
    fn shot_path(&self, shot: Option<&'static str>) -> PathBuf {
        match shot {
            None => self.out_png.clone(),
            Some(n) => {
                let mut stem = self.scratch.out_stem.clone().into_os_string();
                stem.push(format!("_{n}.png"));
                PathBuf::from(stem)
            }
        }
    }

    /// Writes the capture (alpha forced to 255) to `shot_path(shot)`; only `OUT.png` counts as the run's screenshot.
    fn write_png(&mut self, image: &egui::ColorImage, shot: Option<&'static str>) {
        let [w, h] = [image.size[0] as u32, image.size[1] as u32];
        let mut rgba = image.as_raw().to_vec();
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let path = self.shot_path(shot);
        let result = wte_core::encode_png(w, h, &rgba, wte_core::PngLevel::Balanced)
            .map_err(|e| e.to_string())
            .and_then(|png| write_file(&path, &png));
        match result {
            Ok(()) => {
                log::info!("screenshot {w}x{h} written to {}", path.display());
                if shot.is_none() {
                    self.captured_size = Some([w, h]);
                }
            }
            Err(e) => self.fail(EXIT_FAILED, format!("cannot write {}: {e}", path.display())),
        }
    }

    /// Writes `OUT.json`, stores the exit code and closes the window.
    fn finish(&mut self, ctx: &egui::Context, state: &mut AppState, frame: u64) {
        if self.captured_size.is_none() && self.errors.is_empty() {
            self.errors.push("the scenario took no screenshot".to_owned());
            self.code = EXIT_FAILED;
        }
        let ok = self.code == 0 && self.errors.is_empty();
        let report = report_json(
            &self.scenario,
            ok,
            frame,
            self.requested_size,
            self.captured_size.unwrap_or([0, 0]),
            self.size_clamped,
            &self.errors,
            &self.shots,
        );
        if let Err(e) = write_file(&self.out_json, report.as_bytes()) {
            log::error!("cannot write {}: {e}", self.out_json.display());
            if self.code == 0 {
                self.code = EXIT_FAILED;
            }
        }
        self.exit_code.store(self.code, Ordering::SeqCst);
        log::info!("self-test \"{}\" finished: ok = {ok}, exit code {}", self.scenario, self.code);
        log::info!("scratch values: {:?}; scenario files under {}", self.scratch.values, self.scratch.out_stem.display());
        state.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        self.exited_at = Some(frame);
    }
}

/// The step and run timeouts of a scenario: the scenario's own (`scenarios::timeouts`) or the defaults, each overridden
/// by `WTE_SELFTEST_STEP_TIMEOUT` / `WTE_SELFTEST_RUN_TIMEOUT` (whole seconds) when set.
fn timeouts(scenario: &str) -> (Duration, Duration) {
    let (step, run) = scenarios::timeouts(scenario).unwrap_or((STEP_TIMEOUT, RUN_TIMEOUT));
    let env = |name: &str, default: Duration| {
        std::env::var(name).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&s| s > 0).map_or(default, Duration::from_secs)
    };
    (env("WTE_SELFTEST_STEP_TIMEOUT", step), env("WTE_SELFTEST_RUN_TIMEOUT", run))
}

/// Stickers P6a: a `WaitMs` step goes on while `at` has not come.
fn wait_until(at: Instant) -> Option<Active> {
    (Instant::now() < at).then_some(Active::WaitUntil(at))
}

/// `WaitIdle` conditions other than the stable view rects (8.11).
fn is_idle(state: &AppState, tools_busy: bool) -> bool {
    state.busy.is_none()
        && !tools_busy
        && !state.final_bakes_pending
        && !state.wrap_jobs_pending
        && !state.gpu_mips_stale
        && !crate::sticker::live::pending(state)
        && state.doc().is_none_or(|d| !d.has_pending_edit() && !d.has_pending_composite())
}

/// Normalized view coordinates -> screen position.
fn to_screen(rect: egui::Rect, p: [f32; 2]) -> egui::Pos2 {
    rect.min + egui::vec2(p[0] * rect.width(), p[1] * rect.height())
}

/// Hover at the first point, press there, `points_per_frame` moves per frame, release at the last point.
fn pointer_frames(points: &[egui::Pos2], button: egui::PointerButton, mods: egui::Modifiers, points_per_frame: usize) -> Vec<FrameInput> {
    let Some((&first, rest)) = points.split_first() else { return Vec::new() };
    let last = *points.last().unwrap_or(&first);
    let mut frames = vec![
        FrameInput { events: vec![egui::Event::PointerMoved(first)], modifiers: mods, pen: Vec::new() },
        FrameInput {
            events: vec![
                egui::Event::PointerMoved(first),
                egui::Event::PointerButton { pos: first, button, pressed: true, modifiers: mods },
            ],
            modifiers: mods,
            pen: Vec::new(),
        },
    ];
    for chunk in rest.chunks(points_per_frame.max(1)) {
        frames.push(FrameInput { events: chunk.iter().map(|p| egui::Event::PointerMoved(*p)).collect(), modifiers: mods, pen: Vec::new() });
    }
    frames.push(FrameInput {
        events: vec![egui::Event::PointerButton { pos: last, button, pressed: false, modifiers: mods }],
        modifiers: mods,
        pen: Vec::new(),
    });
    frames
}

/// Hover, press, release.
fn click_frames(pos: egui::Pos2, button: egui::PointerButton, mods: egui::Modifiers) -> Vec<FrameInput> {
    pointer_frames(&[pos], button, mods, 1)
}

/// One `PointerMoved`, then in the next frame the wheel event with a whole number of notches.
fn scroll_frames(pos: egui::Pos2, notches: f32, mods: egui::Modifiers) -> Vec<FrameInput> {
    vec![
        FrameInput { events: vec![egui::Event::PointerMoved(pos)], modifiers: egui::Modifiers::NONE, pen: Vec::new() },
        FrameInput {
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, notches),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: mods,
            }],
            modifiers: mods,
            pen: Vec::new(),
        },
    ]
}

/// One `PointerMoved` to `pos`, no button.
fn hover_frames(pos: egui::Pos2) -> Vec<FrameInput> {
    vec![FrameInput { events: vec![egui::Event::PointerMoved(pos)], modifiers: egui::Modifiers::NONE, pen: Vec::new() }]
}

/// Key down in one frame, key up in the next.
fn key_frames(key: egui::Key, mods: egui::Modifiers) -> Vec<FrameInput> {
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods };
    vec![FrameInput { events: vec![event(true)], modifiers: mods, pen: Vec::new() }, FrameInput { events: vec![event(false)], modifiers: mods, pen: Vec::new() }]
}

/// The pointer id and touch device of the self-test's pen (`pen_frames`).
const TEST_PEN_ID: u32 = 77;
const TEST_PEN_DEVICE: egui::TouchDeviceId = egui::TouchDeviceId(0x7e57);

/// The `Touch` event egui-winit pushes for a sample of the self-test's pen.
fn pen_touch(phase: egui::TouchPhase, pos: egui::Pos2, force: Option<f32>) -> egui::Event {
    egui::Event::Touch { device_id: TEST_PEN_DEVICE, id: egui::TouchId(u64::from(TEST_PEN_ID)), phase, pos, force }
}

/// Frames of a recorded pen (branch `pen-input`): per sample the events egui-winit makes for a Windows Ink pen
/// (`on_touch`: `Touch` with winit's force, then the mouse emulation) and the record the window procedure would queue.
/// Hover samples (pressure 0) and contact moves go `points_per_frame` to a frame; the Down, the Up and the final Leave
/// (a record only: winit sends no touch for it) each get a frame of their own, as `pointer_frames` gives the press and
/// the release theirs.
fn pen_frames(points: &[(egui::Pos2, f32)], inverted: bool, barrel: bool, points_per_frame: usize) -> Vec<FrameInput> {
    let per = points_per_frame.max(1);
    let record = |kind: PenKind, pressure: f32| PenRecord {
        pointer_id: TEST_PEN_ID,
        kind,
        in_contact: matches!(kind, PenKind::Down | PenKind::Update) && pressure > 0.0,
        pressure,
        barrel,
        third: false,
        inverted,
    };
    let none = egui::Modifiers::NONE;
    let mut frames: Vec<FrameInput> = Vec::new();
    let mut open: Option<FrameInput> = None;
    let (mut contact, mut last) = (false, None);
    for &(pos, p) in points {
        if p > 0.0 && !contact {
            frames.extend(open.take());
            frames.push(FrameInput {
                events: vec![
                    pen_touch(egui::TouchPhase::Start, pos, Some(p)),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: none },
                ],
                modifiers: none,
                pen: vec![record(PenKind::Down, p)],
            });
            contact = true;
        } else {
            if p <= 0.0 && contact {
                frames.extend(open.take());
                frames.push(pen_up(last.unwrap_or(pos), record(PenKind::Up, 0.0)));
                contact = false;
            }
            let f = open.get_or_insert_with(|| FrameInput { events: Vec::new(), modifiers: none, pen: Vec::new() });
            f.events.push(pen_touch(egui::TouchPhase::Move, pos, (p > 0.0).then_some(p)));
            f.events.push(egui::Event::PointerMoved(pos));
            f.pen.push(record(PenKind::Update, p.max(0.0)));
            if f.pen.len() >= per {
                frames.extend(open.take());
            }
        }
        last = Some(pos);
    }
    frames.extend(open.take());
    if contact && let Some(pos) = last {
        frames.push(pen_up(pos, record(PenKind::Up, 0.0)));
    }
    if !points.is_empty() {
        frames.push(FrameInput { events: Vec::new(), modifiers: none, pen: vec![record(PenKind::Leave, 0.0)] });
    }
    frames
}

/// The Up frame of `pen_frames`: `Touch(End)`, the release at the last pointer position and `PointerGone`.
fn pen_up(pos: egui::Pos2, up: PenRecord) -> FrameInput {
    let none = egui::Modifiers::NONE;
    FrameInput {
        events: vec![
            pen_touch(egui::TouchPhase::End, pos, None),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: none },
            egui::Event::PointerGone,
        ],
        modifiers: none,
        pen: vec![up],
    }
}

/// The JSON report of a run; stickers P6a: `"shots"` lists every named shot with what it must show.
#[expect(clippy::too_many_arguments, reason = "one field per report entry")]
fn report_json(
    scenario: &str,
    ok: bool,
    frames: u64,
    requested_size: [u32; 2],
    size: [u32; 2],
    size_clamped: bool,
    errors: &[String],
    shots: &[(&str, &str)],
) -> String {
    let shots: Vec<serde_json::Value> =
        shots.iter().map(|(name, expect)| serde_json::json!({ "name": name, "expect": expect })).collect();
    let value = serde_json::json!({
        "scenario": scenario,
        "ok": ok,
        "frames": frames,
        "requested_size": requested_size,
        "size": size,
        "size_clamped": size_clamped,
        "errors": errors,
        "shots": shots,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned())
}

/// `path` + `suffix`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Writes a file, creating its directory.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(scenario: &str, out: &str) -> CliArgs {
        CliArgs { path: None, screenshot: Some(PathBuf::from(out)), scenario: scenario.to_owned(), size: [640, 480], log: None }
    }

    #[test]
    fn no_runner_without_screenshot() {
        let a = CliArgs { path: None, screenshot: None, scenario: "smoke".into(), size: [1280, 800], log: None };
        assert!(Runner::new(&a, Arc::new(AtomicI32::new(0))).is_none());
    }

    #[test]
    fn known_and_unknown_scenarios() {
        let code = Arc::new(AtomicI32::new(0));
        let r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::clone(&code)).unwrap();
        assert_eq!(r.out_json, PathBuf::from(r"E:\x\shot.json"));
        assert_eq!(r.scratch.out_stem, PathBuf::from(r"E:\x\shot"));
        assert!(r.errors.is_empty() && r.code == 0);
        assert_eq!(code.load(Ordering::SeqCst), 0);

        let r = Runner::new(&args("no_such_scenario", r"E:\x\shot.png"), Arc::clone(&code)).unwrap();
        assert_eq!(r.code, EXIT_FAILED);
        assert!(r.errors[0].contains("no_such_scenario"));
        assert!(matches!(r.steps.front(), Some(Step::Screenshot)));
        assert_eq!(code.load(Ordering::SeqCst), EXIT_FAILED);
    }

    #[test]
    fn pointer_frames_hover_press_move_release() {
        let pts: Vec<egui::Pos2> = (0..5).map(|i| egui::pos2(i as f32, 0.0)).collect();
        let frames = pointer_frames(&pts, egui::PointerButton::Primary, egui::Modifiers::SHIFT, 2);
        // hover, press, 2 move frames (2 + 2 points), release
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[0].events, vec![egui::Event::PointerMoved(pts[0])]);
        assert!(matches!(frames[1].events[1], egui::Event::PointerButton { pressed: true, pos, .. } if pos == pts[0]));
        assert_eq!(frames[2].events.len(), 2);
        assert_eq!(frames[3].events, vec![egui::Event::PointerMoved(pts[3]), egui::Event::PointerMoved(pts[4])]);
        assert!(matches!(frames[4].events[0], egui::Event::PointerButton { pressed: false, pos, .. } if pos == pts[4]));
        assert!(frames.iter().all(|f| f.modifiers == egui::Modifiers::SHIFT));
        assert!(pointer_frames(&[], egui::PointerButton::Primary, egui::Modifiers::NONE, 1).is_empty());
        assert_eq!(click_frames(pts[1], egui::PointerButton::Secondary, egui::Modifiers::NONE).len(), 3);
    }

    /// Branch `pen-input`: the recorded pen gives what egui-winit and the window procedure give, and through the
    /// router the same pointer events as a mouse drag of the same points (Touch events gone, pressures noted).
    #[test]
    fn pen_frames_hover_down_moves_up_leave() {
        let pts: Vec<(egui::Pos2, f32)> = [(0.0, 0.0), (0.0, 0.5), (1.0, 0.6), (2.0, 0.7), (3.0, 0.8)]
            .iter()
            .map(|&(x, p)| (egui::pos2(x, 0.0), p))
            .collect();
        let frames = pen_frames(&pts, false, false, 2);
        // hover, Down, 2 move frames (2 + 1), Up, Leave
        assert_eq!(frames.len(), 6);
        assert!(matches!(frames[0].events[0], egui::Event::Touch { phase: egui::TouchPhase::Move, force: None, .. }));
        assert_eq!(frames[1].pen[0].kind, PenKind::Down);
        assert_eq!(frames[2].pen.len(), 2);
        assert_eq!(frames[3].pen.len(), 1);
        assert_eq!(frames[4].pen[0].kind, PenKind::Up);
        assert!(frames[5].events.is_empty() && frames[5].pen[0].kind == PenKind::Leave);
        // Through the router: the mouse drag of `pointer_frames` plus the final PointerGone, with the pressures.
        let mut router = crate::pen::PenRouter::default();
        let mut routed = Vec::new();
        let mut notes = Vec::new();
        for f in &frames {
            let mut events = f.events.clone();
            let mut feed = f.pen.iter().copied().collect();
            let frame = router.process(&mut events, &mut feed);
            routed.extend(events);
            notes.extend(frame.pointer.iter().map(|(_, p)| *p));
        }
        let mouse: Vec<egui::Pos2> = pts.iter().map(|(p, _)| *p).collect();
        let mut want: Vec<egui::Event> = pointer_frames(&mouse[1..], egui::PointerButton::Primary, egui::Modifiers::NONE, 2)
            .into_iter()
            .flat_map(|f| f.events)
            .collect();
        want.push(egui::Event::PointerGone);
        assert_eq!(routed, want);
        assert_eq!(notes, vec![None, Some(0.5), Some(0.5), Some(0.6), Some(0.7), Some(0.8), Some(0.8)]);
        // Inverted / barrel go into every record; a sample back at pressure 0 lifts the pen.
        let frames = pen_frames(&[(egui::pos2(0.0, 0.0), 0.4), (egui::pos2(1.0, 0.0), 0.0)], true, true, 4);
        assert!(frames.iter().flat_map(|f| &f.pen).all(|r| r.inverted && r.barrel));
        let kinds: Vec<PenKind> = frames.iter().flat_map(|f| f.pen.iter().map(|r| r.kind)).collect();
        assert_eq!(kinds, vec![PenKind::Down, PenKind::Up, PenKind::Update, PenKind::Leave]);
        assert!(pen_frames(&[], false, false, 1).is_empty());
    }

    #[test]
    fn scroll_and_key_frames() {
        let f = scroll_frames(egui::pos2(3.0, 4.0), 2.0, egui::Modifiers::CTRL);
        assert_eq!(f.len(), 2);
        assert!(matches!(
            f[1].events[0],
            egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. } if delta == egui::vec2(0.0, 2.0)
        ));
        assert_eq!(f[1].modifiers, egui::Modifiers::CTRL);
        let k = key_frames(egui::Key::B, egui::Modifiers::NONE);
        assert!(matches!(k[0].events[0], egui::Event::Key { key: egui::Key::B, pressed: true, .. }));
        assert!(matches!(k[1].events[0], egui::Event::Key { key: egui::Key::B, pressed: false, .. }));
        // Round 4: a hover is one move without a button.
        let h = hover_frames(egui::pos2(5.0, 6.0));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].events, vec![egui::Event::PointerMoved(egui::pos2(5.0, 6.0))]);
        assert_eq!(Step::Hover { view: ViewId::Main3D.into(), at: [0.25, 0.5] }.describe(), "Hover(Main3D, [0.25, 0.5])");
        assert_eq!(Step::Hover { view: Target::StickerCanvas, at: [0.5, 0.5] }.describe(), "Hover(StickerCanvas, [0.5, 0.5])");
        // Stickers P6b: a wheel step at a screen position (a page of the side panel).
        assert_eq!(Step::ScreenScroll { at: egui::pos2(7.0, 8.0), notches: -2.5 }.describe(), "ScreenScroll([7.0 8.0], -2.5)");
    }

    #[test]
    fn a_named_shot_is_written_next_to_out_png() {
        // Gate 1 (round 4): `Step::Shot` keeps a mid-run frame without replacing the run's own screenshot.
        let r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::new(AtomicI32::new(0))).unwrap();
        assert_eq!(r.shot_path(None), PathBuf::from(r"E:\x\shot.png"));
        assert_eq!(r.shot_path(Some("hover")), PathBuf::from(r"E:\x\shot_hover.png"));
        assert_eq!(Step::Shot { name: "hover", expect: "" }.describe(), "Shot(hover)");
    }

    #[test]
    fn hook_replaces_real_input_and_sets_time() {
        let mut r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::new(AtomicI32::new(0))).unwrap();
        r.frame = 30;
        r.inject.extend(key_frames(egui::Key::B, egui::Modifiers::SHIFT));
        let ctx = egui::Context::default();
        let real = || egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                egui::Event::PointerMoved(egui::pos2(1.0, 1.0)),
                egui::Event::Text("x".into()),
                egui::Event::Copy,
            ],
            ..Default::default()
        };
        let mut raw = real();
        r.raw_input_hook(&ctx, &mut raw);
        assert_eq!(raw.events.len(), 2);
        assert_eq!(raw.events[0], egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
        assert!(matches!(raw.events[1], egui::Event::Key { key: egui::Key::B, pressed: true, .. }));
        assert_eq!(raw.time, Some(0.5));
        assert_eq!(r.last_inject_frame, Some(30));
        // Same modifiers in the key-up frame: not announced again.
        let mut raw = real();
        r.raw_input_hook(&ctx, &mut raw);
        assert_eq!(raw.events.len(), 1);
        assert!(matches!(raw.events[0], egui::Event::Key { key: egui::Key::B, pressed: false, .. }));
        // Nothing injected: modifiers go back to NONE once, the real events stay dropped.
        let mut raw = real();
        r.raw_input_hook(&ctx, &mut raw);
        assert_eq!(raw.events, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        let mut raw = real();
        r.raw_input_hook(&ctx, &mut raw);
        assert!(raw.events.is_empty());
    }

    #[test]
    fn failures_skip_to_capture_and_exit() {
        let code = Arc::new(AtomicI32::new(0));
        let mut r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::clone(&code)).unwrap();
        r.fail(EXIT_FAILED, "boom".into());
        assert_eq!(code.load(Ordering::SeqCst), EXIT_FAILED);
        assert!(matches!(r.steps.iter().collect::<Vec<_>>()[..], [Step::Screenshot, Step::Exit]));
        r.fail(EXIT_TIMEOUT, "again".into());
        assert_eq!(r.code, EXIT_FAILED, "the first failure decides the exit code");
        assert!(matches!(r.steps.iter().collect::<Vec<_>>()[..], [Step::Exit]));
        assert_eq!(r.errors, ["boom", "again"]);
    }

    #[test]
    fn report_has_the_contract_fields() {
        let json = report_json("smoke", true, 42, [1280, 800], [1280, 800], false, &[], &[("a", "the model")]);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["scenario"], "smoke");
        assert_eq!(v["ok"], true);
        assert_eq!(v["frames"], 42);
        assert_eq!(v["requested_size"], serde_json::json!([1280, 800]));
        assert_eq!(v["size"], serde_json::json!([1280, 800]));
        assert_eq!(v["size_clamped"], false);
        assert_eq!(v["errors"], serde_json::json!([]));
        assert_eq!(v["shots"], serde_json::json!([{ "name": "a", "expect": "the model" }]));
    }

    /// Stickers P6a: a step aimed at the sticker canvas finds its rect in `AppState::sticker_canvas_rect` and fails
    /// clearly when the canvas is not shown; a view target reads `view_rects` as before.
    #[test]
    fn targets_resolve_to_the_view_or_the_sticker_canvas() {
        let mut r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::new(AtomicI32::new(0))).unwrap();
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        let canvas = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 200.0));
        let view = egui::Rect::from_min_size(egui::pos2(320.0, 20.0), egui::vec2(300.0, 200.0));
        state.view_rects.insert(ViewId::Main3D, view);
        assert_eq!(r.view_rect(&state, ViewId::Main3D.into(), "probe"), Some(view));
        assert!(r.errors.is_empty());
        assert_eq!(r.view_rect(&state, Target::StickerCanvas, "probe"), None);
        assert!(r.errors[0].contains("StickerCanvas"), "{:?}", r.errors);
        state.sticker_canvas_rect = Some(canvas);
        assert_eq!(r.view_rect(&state, Target::StickerCanvas, "probe"), Some(canvas));
        assert_eq!(Target::from(ViewId::Main2D), Target::View(ViewId::Main2D));
    }

    /// Stickers P6a (review A14): `WaitMs` waits wall-clock time, however many frames pass; a named shot is listed
    /// with its `expect`.
    #[test]
    fn wait_ms_waits_wall_clock_time_and_shots_keep_their_expectation() {
        let mut r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::new(AtomicI32::new(0))).unwrap();
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        let ctx = egui::Context::default();
        r.steps = VecDeque::from([Step::WaitMs(60), Step::WaitMs(0)]);
        let start = Instant::now();
        r.start_next(&ctx, &mut state, 1);
        let Some((Active::WaitUntil(at), _)) = r.active.take() else { panic!("WaitMs is a WaitUntil step") };
        let mut frames = 0u64;
        while wait_until(at).is_some() {
            frames += 1;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(start.elapsed() >= Duration::from_millis(60), "ended after {:?}", start.elapsed());
        assert!(frames >= 2, "{frames} polls");
        r.start_next(&ctx, &mut state, 2);
        assert!(matches!(r.active.as_ref().map(|a| &a.0), Some(Active::WaitUntil(_))));
        assert_eq!(Step::WaitMs(3000).describe(), "WaitMs(3000)");
        r.active = None;
        r.steps = VecDeque::from([Step::Shot { name: "canvas", expect: "the photo on a checkerboard" }]);
        r.start_next(&ctx, &mut state, 3);
        assert_eq!(r.shots, [("canvas", "the photo on a checkerboard")]);
    }

    #[test]
    fn idle_needs_no_session_work() {
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        assert!(is_idle(&state, false));
        assert!(!is_idle(&state, true));
        state.gpu_mips_stale = true;
        assert!(!is_idle(&state, false));
        state.gpu_mips_stale = false;
        state.busy = Some(crate::state::BusyInfo { title: "Fonts".into(), stage: String::new(), fraction: 0.0, modal: false });
        assert!(!is_idle(&state, false));
        // Round 5: a pending Final stamp bake is work too.
        state.busy = None;
        assert!(is_idle(&state, false));
        state.final_bakes_pending = true;
        assert!(!is_idle(&state, false));
    }

    #[test]
    fn do_steps_wait_for_pending_bakes_unless_switched_off() {
        let mut r = Runner::new(&args("smoke", r"E:\x\shot.png"), Arc::new(AtomicI32::new(0))).unwrap();
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        let ctx = egui::Context::default();
        // A Do step is followed by the bake wait.
        r.steps = VecDeque::from([Step::Do(AppAction::Undo), Step::BakeWait(false), Step::Do(AppAction::Redo)]);
        r.start_next(&ctx, &mut state, 10);
        assert_eq!(state.actions.len(), 1);
        assert!(matches!(r.active, Some((Active::Bakes, _))));
        assert!(!r.waits_for_bakes(&state), "nothing pending: the wait ends in the next frame");
        state.final_bakes_pending = true;
        assert!(r.waits_for_bakes(&state));
        // BakeWait(false): no wait after Do, and input steps do not wait either.
        r.active = None;
        r.start_next(&ctx, &mut state, 11);
        assert!(!r.bake_wait && !r.waits_for_bakes(&state));
        r.start_next(&ctx, &mut state, 12);
        assert_eq!(state.actions.len(), 2);
        assert!(r.active.is_none(), "no wait after Do with BakeWait(false)");
        // WaitIdle still waits.
        assert!(!is_idle(&state, false));
        assert_eq!(Step::BakeWait(true).describe(), "BakeWait(true)");
    }
}
