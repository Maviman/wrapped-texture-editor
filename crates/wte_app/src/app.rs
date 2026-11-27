// Owner: app-shell (A1). `WteApp` and the frame order of ARCHITECTURE.md 8.3.

use std::sync::Arc;
use std::sync::atomic::AtomicI32;

use eframe::{egui, wgpu};

use crate::action::AppAction;
use crate::cli::CliArgs;
use crate::gpu::{Gpu, GpuScene, Renderer, ViewTargets};
use crate::jobs::Jobs;
use crate::state::{AppState, ExportDialogState, Prefs};
use crate::tools::ToolController;
use crate::view::ViewCx;
use crate::{dialogs, dispatch, dnd, io_flows, panels, prefs, selftest, shortcuts, theme, view};

/// The application: state, GPU objects, tools, UI caches, jobs and the optional self-test runner.
pub struct WteApp {
    pub state: AppState,
    pub gpu: Gpu,
    pub renderer: Renderer,
    pub scene: Option<GpuScene>,
    pub targets: ViewTargets,
    pub tools: ToolController,
    pub panels: panels::PanelState,
    pub dialogs: dialogs::DialogsState,
    pub jobs: Jobs,
    pub selftest: Option<selftest::Runner>,
    /// Branch `companion`: the Tablet companion server (off unless a link is on; ARCHITECTURE 8.14).
    pub companion: crate::companion::Companion,
    pub exit_code: Arc<AtomicI32>,
    /// The egui context (jobs repaint through it; dispatch sends viewport commands with it).
    pub(crate) ctx: egui::Context,
    /// The Export dialog, parked while the `Dialog::Message` of a failed export is shown; restored when that message
    /// closes, so the user can retry (8.9: "the Export dialog stays open underneath").
    pub(crate) export_stash: Option<Box<ExportDialogState>>,
    /// fix_export item 6: an export (polish item 2: or a save) waiting behind its progress dialog for the Final bakes
    /// to land (`io_flows::feed_waiting_snapshot` hands its job the snapshot then).
    pub(crate) snapshot_feed: Option<io_flows::SnapshotFeed>,
    /// `pixels_per_point` the window's maximum inner size was last sent for (`cap_window_size`).
    max_size_ppp: Option<f32>,
    /// Frames the start-up check of the window geometry is still waiting for (`fit_window_to_monitor`); 0 = done.
    geometry_frames_left: u32,
    /// Frames drawn so far (only for the debug-build panic hook test, `WTE_DEBUG_PANIC_FRAME`).
    #[cfg(debug_assertions)]
    frames: u64,
}

/// Set by the uncaptured-error handler when `Surface::configure` failed (window larger than the device's surface limit):
/// `cap_window_size` shrinks the window.
static SURFACE_TOO_LARGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// True from such a failure until the window is within the limit again; meanwhile the wgpu errors of egui's pass for the
/// oversized window are logged instead of being fatal.
static OVERSIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Frames the start-up geometry check waits at most for the monitor size to be known.
const GEOMETRY_CHECK_FRAMES: u32 = 30;
/// Share of the monitor a fitted window may take (room for the taskbar and the title bar).
const MONITOR_SHARE: egui::Vec2 = egui::vec2(0.96, 0.9);

/// A window larger than its monitor or partly off it at start-up (a remembered size from a larger monitor or a higher
/// DPI): the inner size that fits, and where to put the window (centred on a monitor at the origin). `None` when the
/// window fits. `outer` / `inner` are the window rects and `monitor` the monitor size, all in points; a window whose
/// top-left corner is not on a monitor at the origin (left of or above the primary monitor) is only resized.
pub(crate) fn fit_to_monitor(outer: egui::Rect, inner: egui::Rect, monitor: egui::Vec2) -> Option<(egui::Vec2, Option<egui::Pos2>)> {
    if !(monitor.x > 0.0 && monitor.y > 0.0 && outer.is_finite() && inner.is_finite()) {
        return None;
    }
    let deco = (outer.size() - inner.size()).max(egui::Vec2::ZERO);
    let room = monitor * MONITOR_SHARE - deco;
    let size = inner.size().min(room).max(egui::vec2(1.0, 1.0));
    let resized = size != inner.size();
    let on_origin_monitor = outer.min.x >= 0.0 && outer.min.y >= 0.0 && outer.min.x < monitor.x && outer.min.y < monitor.y;
    let off_screen = outer.min.x + deco.x + size.x > monitor.x || outer.min.y + deco.y + size.y > monitor.y * MONITOR_SHARE.y + deco.y;
    let position = (on_origin_monitor && off_screen).then(|| ((monitor - (size + deco)) * 0.5).max(egui::Vec2::ZERO).to_pos2());
    (resized || position.is_some()).then_some((size, position))
}

impl WteApp {
    /// Builds the app inside eframe's app creator: applies the theme, builds `Gpu` / `Renderer` / `ViewTargets`, loads
    /// `Prefs` (not in self-test), starts the Fonts job, creates the self-test `Runner` when `args.screenshot` is set
    /// and pushes `OpenPath(path)` for `args.path`. `Err` when there is no wgpu render state.
    pub fn new(cc: &eframe::CreationContext<'_>, args: CliArgs, exit_code: Arc<AtomicI32>) -> anyhow::Result<WteApp> {
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no wgpu render state is available (no usable graphics adapter)"))?;
        let ctx = cc.egui_ctx.clone();
        theme::apply(&ctx);
        let info = rs.adapter.get_info();
        log::info!("graphics adapter: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        crate::crash::log_startup_diagnostics(&format!("{:?} on {} (driver {} {})", info.backend, info.name, info.driver, info.driver_info));
        crate::crash::reassert_filter();
        crate::crash::debug_trigger_from_env();

        let gpu = Gpu::new(rs);
        log::info!("view targets: {}x MSAA", gpu.msaa);
        // Pipeline creation failures (an unsupported backend) become a startup error with a message instead of a
        // wgpu validation panic.
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer::new(&gpu);
        let targets = ViewTargets::new(&gpu);
        if let Some(error) = poll_ready(scope.pop()) {
            anyhow::bail!("the graphics adapter {} ({:?}) cannot run the renderer: {error}", info.name, info.backend);
        }
        // A surface larger than the device allows (a window resized programmatically past the MaxInnerSize cap of
        // `cap_window_size`, which only limits user resizing) must not end the app: egui-wgpu keeps the previous surface
        // configuration, and `logic` shrinks the window back below the limit. Every other uncaptured wgpu error stays
        // fatal, as wgpu's default handler makes it (a panic: the hook's error box and exit 101).
        gpu.device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            use std::sync::atomic::Ordering::SeqCst;
            let text = error.to_string();
            if text.contains("Surface::configure") {
                SURFACE_TOO_LARGE.store(true, SeqCst);
                if !OVERSIZED.swap(true, SeqCst) {
                    log::warn!("the window surface cannot be configured; the window is made smaller: {text}");
                }
                return;
            }
            if OVERSIZED.load(SeqCst) {
                // egui-wgpu's own pass for the oversized window fails the same way until the window is smaller again.
                log::warn!("wgpu error while the window is larger than the surface limit (ignored): {}", text.lines().last().unwrap_or(""));
                return;
            }
            log::error!("wgpu error: {text}");
            panic!("wgpu error: {text}");
        }));
        // A lost device (driver reset, TDR) gives a clear message and exit instead of a later panic.
        gpu.device.set_device_lost_callback(|reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                log::error!("graphics device lost ({reason:?}): {message}");
                crate::fatal_exit(
                    &format!(
                        "The graphics device was lost ({message}). Unsaved changes could not be kept.\n\
                         Updating the graphics driver may help; WGPU_BACKEND=vulkan or dx12 selects another backend."
                    ),
                    crate::EXIT_PANIC,
                );
            }
        });

        let selftest_mode = args.screenshot.is_some();
        if selftest_mode || std::env::var_os("WTE_FRAME_PROFILE").is_some() {
            frame_profile::enable();
        }
        let loaded_prefs = if selftest_mode { Prefs::default() } else { prefs::load() };
        let mut state = AppState::new(loaded_prefs, selftest_mode);
        // Round 5 (R5-1): Final stamp bakes run on a worker; `WTE_SYNC_FINAL_BAKES=1` keeps them on the UI thread (the
        // round-4 behaviour, for measurements and A/B comparisons).
        state.async_final_bakes = std::env::var("WTE_SYNC_FINAL_BAKES").map_or(true, |v| v.trim() != "1");
        log::info!("final stamp bakes: {}", if state.async_final_bakes { "on a worker" } else { "on the UI thread (WTE_SYNC_FINAL_BAKES=1)" });
        // Round 6 (R6-2): `WTE_STAMP_MODE=wrap` starts the Stamp / Text tools in Wrap mode (a measuring aid: the same
        // clicks then place wrapped stamps); the default is Project.
        state.tools.stamp.mode = crate::state::stamp_mode_from_env(std::env::var("WTE_STAMP_MODE").ok().as_deref());
        log::info!(
            "new 3D stamps: {}",
            if state.tools.stamp.mode == wte_core::SurfaceMode::Wrap { "Wrap (WTE_STAMP_MODE=wrap)" } else { "Project" }
        );
        // Branch `pen-input` (part D): the window-procedure subclass for pen flags, per-sample pressure and no Ink
        // feedback; in a self-test run only with WTE_PEN_SUBCLASS=1 (to show that it does no harm). Without it an
        // interactive run takes every touch for a pen with winit's pressure.
        let pen_subclass = std::env::var("WTE_PEN_SUBCLASS").is_ok_and(|v| v.trim() == "1");
        if !selftest_mode || pen_subclass {
            let installed = crate::pen::install(cc);
            state.pen.router.touch_is_pen = !installed && !selftest_mode;
        }
        let runner = selftest::Runner::new(&args, Arc::clone(&exit_code));
        if let Some(path) = args.path.clone() {
            state.push(AppAction::OpenPath(path));
        }

        let panel_state = panels::PanelState::with_prefs(&state.prefs);
        let mut app = WteApp {
            state,
            gpu,
            renderer,
            scene: None,
            targets,
            tools: ToolController::new(),
            panels: panel_state,
            dialogs: dialogs::DialogsState::new(),
            jobs: Jobs::new(),
            selftest: runner,
            companion: crate::companion::Companion::new(selftest_mode),
            exit_code,
            ctx,
            export_stash: None,
            snapshot_feed: None,
            max_size_ppp: None,
            geometry_frames_left: if selftest_mode { 0 } else { GEOMETRY_CHECK_FRAMES },
            #[cfg(debug_assertions)]
            frames: 0,
        };
        io_flows::start_fonts(&mut app);
        crate::brush_flows::startup(&mut app);
        // Branch `companion`: links the user left on start again (never in a self-test run, C13).
        if !selftest_mode {
            let ctx = app.ctx.clone();
            app.companion.apply_prefs(&ctx, &mut app.state.prefs.companion);
        }
        Ok(app)
    }
}

/// The value of a wgpu future that is ready at once (native wgpu resolves error scopes synchronously); `None` when it
/// is not ready.
fn poll_ready<F: std::future::Future>(f: F) -> F::Output
where
    F::Output: Default,
{
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match f.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => Default::default(),
    }
}

impl WteApp {
    /// Caps the window's inner size at the largest surface the device accepts (`max_texture_dimension_2d` physical
    /// pixels): a window stretched over several monitors would otherwise make `Surface::configure` fail, which is a
    /// panic. Sent again when `pixels_per_point` changes (the command is converted with it).
    fn cap_window_size(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let limit_px = self.gpu.limits().max_texture_dimension_2d as f32;
        if ppp.is_finite() && ppp > 0.0 {
            let inner = ctx.input(|i| i.viewport().inner_rect);
            if SURFACE_TOO_LARGE.swap(false, std::sync::atomic::Ordering::SeqCst)
                && let Some(inner) = inner
            {
                // The surface could not be configured for the current size: shrink the window below the limit.
                let max = (limit_px / ppp - 16.0).max(64.0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(inner.size().min(egui::Vec2::splat(max))));
            }
            if inner.is_some_and(|r| r.width() * ppp <= limit_px && r.height() * ppp <= limit_px) {
                OVERSIZED.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        if self.max_size_ppp == Some(ppp) || !ppp.is_finite() || ppp <= 0.0 {
            return;
        }
        self.max_size_ppp = Some(ppp);
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::Vec2::splat(limit_px / ppp)));
    }

    /// Once at start-up (not in self-test mode): a restored window that is larger than its monitor or partly off it
    /// is shrunk and centred (`fit_to_monitor`); a maximized one is left alone.
    fn fit_window_to_monitor(&mut self, ctx: &egui::Context) {
        if self.geometry_frames_left == 0 {
            return;
        }
        self.geometry_frames_left -= 1;
        let (outer, inner, monitor, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.monitor_size, v.maximized.unwrap_or(false))
        });
        let (Some(outer), Some(inner), Some(monitor)) = (outer, inner, monitor) else { return };
        self.geometry_frames_left = 0;
        if maximized {
            return;
        }
        if let Some((size, position)) = fit_to_monitor(outer, inner, monitor) {
            log::info!("fitting the window ({:?}) onto its monitor ({monitor:?}): {size:?}", inner.size());
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if let Some(pos) = position {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
        }
    }

    /// Debug builds only: `WTE_DEBUG_PANIC_FRAME=<n>` panics on the UI thread at frame `n`, to check that the panic
    /// hook shows its box and ends the process (8.3).
    #[cfg(debug_assertions)]
    fn debug_panic_hook_test(&mut self) {
        self.frames += 1;
        let at = std::env::var("WTE_DEBUG_PANIC_FRAME").ok().and_then(|v| v.parse::<u64>().ok());
        if at == Some(self.frames) {
            panic!("WTE_DEBUG_PANIC_FRAME: test panic on the UI thread at frame {}", self.frames);
        }
    }
}

/// Shows the central view panel and returns the area it was given. Not its response rect: that one grows with
/// content that overflows the panel (a long row of texture-set tabs in the Split view's 2D half), and toasts placed in
/// it would then reach under the side panel (round-3 gate 2, tex_formats.obj).
fn show_central(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    egui::CentralPanel::no_frame()
        .show(ui, |ui| {
            let area = ui.max_rect();
            add_contents(ui);
            area
        })
        .inner
}

impl eframe::App for WteApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Branch `pen-input` (ARCHITECTURE 8.3): (1) the self-test's injected input and pen records, or (2) the pen
        // records of the window procedure; (3) the pen router pairs them with the frame's events.
        if let Some(runner) = &mut self.selftest {
            let records = runner.raw_input_hook(ctx, raw_input);
            self.state.pen.feed.extend(records);
            // A real pen must not reach a scenario (`WTE_PEN_SUBCLASS=1` installs the subclass in a self-test run).
            crate::pen::discard_window_records();
        } else {
            crate::pen::drain_window_records(&mut self.state.pen.feed);
        }
        // Branch `companion` (ARCHITECTURE 8.14): the tablet's input, in-process; nothing without a connected tablet.
        self.companion.inject(ctx, raw_input, &mut self.state);
        self.state.pen.route(&mut raw_input.events);
        // Part G: the opt-in WinTab pressure (interactive runs only; nothing unless its context gives packets).
        if self.selftest.is_none() {
            self.state.pen.apply_wintab(&raw_input.events);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        frame_profile::frame_begin();
        self.state.frame_work.begin();
        // Branch `pen-input` (decision D7): the pen's eraser end switches to the Eraser and back, through the normal
        // `SetTool` dispatch, before the tools and panels see this frame's input.
        // Stickers P6a: in the Sticker view the eraser end selects the sticker Eraser (`SetStickerTool`) instead.
        if self.state.view_mode == crate::state::ViewMode::Sticker {
            let busy = self.tools.is_busy() || self.state.sticker_ui.busy();
            let tool = crate::sticker::tool_of(&self.state);
            if let Some(tool) = crate::pen::sticker_eraser_override(&mut self.state.pen, tool, busy) {
                dispatch::dispatch(self, AppAction::SetStickerTool(tool), frame);
            }
        } else if let Some(tool) = crate::pen::eraser_override(&mut self.state.pen, self.state.tool, self.tools.is_busy()) {
            dispatch::dispatch(self, AppAction::SetTool(tool), frame);
        }
        // Part G: the "Tablet input" preference takes effect at once (never in a self-test run).
        if !self.state.selftest
            && let Some(text) = self.state.pen.sync_tablet_input(self.state.prefs.tablet_input)
        {
            self.state.toasts.info(text);
        }
        let jobs = frame_profile::scope("jobs");
        for done in self.jobs.poll() {
            io_flows::on_job_output(self, done.output, done.cancel_requested); // may replace the session
        }
        // Round 5 (R5-1): apply a finished Final stamp bake and start the next one.
        crate::stamp_bakes::pump(self);
        // Round 6 (R6-2): a selected wrapped stamp whose parameterization is not cached (after open / undo / redo)
        // gets it before the panels and views draw its stats line and outline. Round 8 (R8-2): in the asynchronous mode
        // the tool asks for a param-only job instead (`stamp_bakes`), started here.
        self.tools.ensure_wrap_param(&mut self.state);
        crate::stamp_bakes::after_tools(&mut self.state, &mut self.jobs, ctx);
        // fix_export item 6: an export (polish item 2: or a save) that waits for the Final bakes gets its snapshot once
        // they have landed.
        io_flows::feed_waiting_snapshot(self);
        // Stickers P9a (STICKER_PLAN 5.6): the editor link's watcher events, held updates and questions.
        crate::link::pump(&mut self.state, ctx, self.tools.is_busy());
        self.state.busy = self.jobs.busy();
        drop(jobs);
        // Branch `companion`: events, STATE, the capture of this frame, undo / redo from the tablet.
        self.companion.logic(ctx, &mut self.state, &self.gpu.device, self.tools.is_busy());
        if let Some(runner) = &mut self.selftest {
            let _p = frame_profile::scope("selftest");
            runner.tick(ctx, &mut self.state, &self.gpu, self.tools.is_busy());
        }
        io_flows::handle_close_request(self, ctx);
        io_flows::update_title(self, ctx);
        self.cap_window_size(ctx);
        self.fit_window_to_monitor(ctx);
        #[cfg(debug_assertions)]
        self.debug_panic_hook_test();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let p = frame_profile::scope("menu/status/toolbar");
        let late = shortcuts::handle(ui.ctx(), &mut self.state);
        egui::Panel::top("menu").show(ui, |ui| panels::show_menu_bar(ui, &mut self.state));
        egui::Panel::bottom("status").show(ui, |ui| panels::show_status_bar(ui, &mut self.state));
        egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| panels::show_toolbar(ui, &mut self.state));
        drop(p);
        let p = frame_profile::scope("side panel");
        let side = egui::Panel::right("side")
            .default_size(300.0)
            .show(ui, |ui| panels::show_side_panel(ui, &mut self.state, &mut self.panels));
        self.state.side_panel_rect = Some(side.response.rect);
        drop(p);
        let mut cx = ViewCx {
            state: &mut self.state,
            gpu: &self.gpu,
            renderer: &mut self.renderer,
            scene: &mut self.scene,
            targets: &mut self.targets,
            tools: &mut self.tools,
        };
        let p = frame_profile::scope("central");
        let central = show_central(ui, |ui| view::central(ui, &mut cx));
        cx.state.central_rect = Some(central);
        drop(p);
        let p = frame_profile::scope("dialogs/toasts");
        dialogs::show(ui.ctx(), &mut cx, &mut self.dialogs);
        // Toasts sit in the central view area's corner, never over the side panel or the status bar (8.9).
        self.state.toasts.show(ui.ctx(), central);
        dnd::handle(ui.ctx(), &mut self.state);
        drop(p);
        // Shortcut actions go last: a text field that lost focus because of the shortcut (8.10) has by now queued its
        // Commit::Final action, which must be dispatched before e.g. Save.
        self.state.actions.extend(late);
        let p = frame_profile::scope("dispatch");
        for action in std::mem::take(&mut self.state.actions) {
            dispatch::dispatch(self, action, frame);
        }
        // Round 5 (R5-1): a Final of this frame gets its bake started in the next one, even without input.
        crate::stamp_bakes::after_dispatch(&mut self.state, ui.ctx());
        drop(p);
        frame_profile::frame_end();
        self.state.frame_work.end();
    }

    fn on_exit(&mut self) {
        if self.jobs.any_running() {
            log::info!("exiting while background jobs are still running");
        }
        if !self.state.selftest {
            crate::brush_flows::sync_prefs(&mut self.state);
            panels::sync_prefs(&mut self.state, &self.panels);
            prefs::save(&self.state.prefs);
        }
        log::info!("exit (exit code {})", self.exit_code.load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[cfg(test)]
impl WteApp {
    /// A complete app on a given (headless) device for the GPU control-sweep tests: self-test mode (no native file
    /// dialogs, no prefs), no fonts job, no scenario runner.
    pub(crate) fn for_tests(ctx: egui::Context, gpu: Gpu) -> WteApp {
        theme::apply(&ctx);
        let renderer = Renderer::new(&gpu);
        let targets = ViewTargets::new(&gpu);
        WteApp {
            state: AppState::new(Prefs::default(), true),
            gpu,
            renderer,
            scene: None,
            targets,
            tools: ToolController::new(),
            panels: panels::PanelState::new(),
            dialogs: dialogs::DialogsState::new(),
            jobs: Jobs::new(),
            selftest: None,
            companion: crate::companion::Companion::new(true),
            exit_code: Arc::new(AtomicI32::new(0)),
            ctx,
            export_stash: None,
            snapshot_feed: None,
            max_size_ppp: None,
            geometry_frames_left: 0,
            #[cfg(debug_assertions)]
            frames: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    /// Gate 2 (tex_formats.obj): a central view whose content is wider than the panel (a long row of set tabs) still
    /// reports the area it was given, so the toasts placed in it stay off the side panel.
    #[test]
    fn central_area_ignores_content_that_overflows_into_the_side_panel() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            ..Default::default()
        };
        let (mut side, mut central, mut grown) = (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
        for _ in 0..2 {
            crate::theme::run_test_pass(&ctx, input.clone(), |ui| {
                side = egui::Panel::right("side").exact_size(300.0).show(ui, |ui| ui.label("side")).response.rect;
                central = show_central(ui, |ui| {
                    ui.horizontal(|ui| ui.add_sized(egui::vec2(1400.0, 20.0), egui::Label::new("set tabs")));
                    grown = ui.min_rect();
                });
            });
        }
        assert!(grown.right() > side.left(), "the test content must overflow: {grown:?} vs side panel {side:?}");
        assert!(central.right() <= side.left() + 0.5, "central area {central:?} reaches the side panel {side:?}");
        assert!(central.width() > 800.0, "central area {central:?}");
        // The response rect used before this fix does grow with that content.
        let ctx = egui::Context::default();
        let mut response = egui::Rect::NOTHING;
        for _ in 0..2 {
            crate::theme::run_test_pass(&ctx, input.clone(), |ui| {
                side = egui::Panel::right("side").exact_size(300.0).show(ui, |ui| ui.label("side")).response.rect;
                response = egui::CentralPanel::no_frame()
                    .show(ui, |ui| {
                        ui.horizontal(|ui| ui.add_sized(egui::vec2(1400.0, 20.0), egui::Label::new("set tabs")));
                    })
                    .response
                    .rect;
            });
        }
        assert!(response.right() > side.left(), "old response rect {response:?} vs side panel {side:?}");
    }

    #[test]
    fn a_window_larger_than_its_monitor_is_shrunk_and_centred() {
        let monitor = egui::vec2(1920.0, 1080.0);
        // The case of the review: a maximized size restored as a normal window at the default cascade position.
        let inner = rect(242.0, 265.0, 1920.0, 1129.0);
        let outer = rect(234.0, 234.0, 1936.0, 1168.0);
        let (size, pos) = fit_to_monitor(outer, inner, monitor).expect("does not fit");
        let deco = outer.size() - inner.size();
        assert!(size.x + deco.x <= monitor.x * MONITOR_SHARE.x + 0.01 && size.y + deco.y <= monitor.y * MONITOR_SHARE.y + 0.01);
        let pos = pos.expect("moved onto the monitor");
        assert!(pos.x >= 0.0 && pos.y >= 0.0);
        assert!(pos.x + size.x + deco.x <= monitor.x && pos.y + size.y + deco.y <= monitor.y);
        // A window that fits stays where it is.
        assert_eq!(fit_to_monitor(rect(100.0, 100.0, 1296.0, 839.0), rect(108.0, 131.0, 1280.0, 800.0), monitor), None);
        // Fits in size but hangs off the bottom right: only moved.
        let (size, pos) = fit_to_monitor(rect(1000.0, 600.0, 1296.0, 839.0), rect(1008.0, 631.0, 1280.0, 800.0), monitor).unwrap();
        assert_eq!(size, egui::vec2(1280.0, 800.0));
        assert!(pos.is_some());
        // On a monitor left of the primary one (negative x) the position is not guessed, only the size is fitted.
        let (_, pos) = fit_to_monitor(rect(-2500.0, 10.0, 2600.0, 900.0), rect(-2492.0, 41.0, 2584.0, 861.0), monitor).unwrap();
        assert!(pos.is_none());
        assert_eq!(fit_to_monitor(outer, inner, egui::Vec2::ZERO), None);
    }
}

/// Frame-time profile (ARCHITECTURE.md 8.3, "Frame profile"): CPU time of the named segments of each frame and the
/// interval between frames, summarised in the log every `WINDOW`. On in self-test mode and with `WTE_FRAME_PROFILE`
/// set; otherwise every call is a cheap no-op. UI thread only.
pub(crate) mod frame_profile {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    /// Length of one summary window.
    const WINDOW: Duration = Duration::from_secs(2);

    #[derive(Default)]
    struct Segment {
        name: &'static str,
        total: Duration,
        max: Duration,
        /// Time of the current frame (a segment may run several times per frame).
        frame: Duration,
    }

    struct Profile {
        segments: Vec<Segment>,
        window_start: Instant,
        frames: u32,
        frame_start: Option<Instant>,
        last_frame_start: Option<Instant>,
        interval_total: Duration,
        interval_max: Duration,
        cpu_total: Duration,
        cpu_max: Duration,
        /// Round 4 (gate): `WTE_FRAME_LOG` was set at start: one `frame timing:` line per frame (the measurements of
        /// FEATURE_BRUSH P11 / P13 and FEATURE_TEXT 8.4 need single frames, not the 2-second windows).
        per_frame: bool,
        /// Interval from the previous frame's start to this frame's start (zero for the first frame).
        last_interval: Duration,
    }

    thread_local! {
        static PROFILE: RefCell<Option<Profile>> = const { RefCell::new(None) };
    }

    /// Turns the profile on for this (the UI) thread.
    pub(crate) fn enable() {
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            if p.is_none() {
                *p = Some(Profile {
                    segments: Vec::new(),
                    window_start: Instant::now(),
                    frames: 0,
                    frame_start: None,
                    last_frame_start: None,
                    interval_total: Duration::ZERO,
                    interval_max: Duration::ZERO,
                    cpu_total: Duration::ZERO,
                    cpu_max: Duration::ZERO,
                    per_frame: std::env::var_os("WTE_FRAME_LOG").is_some(),
                    last_interval: Duration::ZERO,
                });
            }
        });
    }

    /// True when the profile is on.
    pub(crate) fn enabled() -> bool {
        PROFILE.with(|p| p.borrow().is_some())
    }

    /// Times one segment until the guard is dropped.
    pub(crate) struct Scope {
        name: &'static str,
        start: Option<Instant>,
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            if let Some(start) = self.start {
                add(self.name, start.elapsed());
            }
        }
    }

    /// Starts timing segment `name` (a no-op guard when the profile is off).
    pub(crate) fn scope(name: &'static str) -> Scope {
        Scope { name, start: enabled().then(Instant::now) }
    }

    fn add(name: &'static str, d: Duration) {
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            let Some(p) = p.as_mut() else { return };
            let index = match p.segments.iter().position(|s| s.name == name) {
                Some(i) => i,
                None => {
                    p.segments.push(Segment { name, ..Default::default() });
                    p.segments.len() - 1
                }
            };
            p.segments[index].frame += d;
        });
    }

    /// Start of a frame (`logic`).
    pub(crate) fn frame_begin() {
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            let Some(p) = p.as_mut() else { return };
            let now = Instant::now();
            if let Some(last) = p.last_frame_start {
                let d = now - last;
                p.interval_total += d;
                p.interval_max = p.interval_max.max(d);
                p.last_interval = d;
            }
            p.last_frame_start = Some(now);
            p.frame_start = Some(now);
        });
    }

    /// End of a frame (end of `ui`): folds the frame into the window; logs and restarts the window when it is full.
    pub(crate) fn frame_end() {
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            let Some(p) = p.as_mut() else { return };
            let Some(start) = p.frame_start.take() else { return };
            let cpu = start.elapsed();
            p.frames += 1;
            p.cpu_total += cpu;
            p.cpu_max = p.cpu_max.max(cpu);
            if p.per_frame {
                log::info!("{}", frame_line(p.last_interval, cpu, &p.segments));
            }
            for s in &mut p.segments {
                s.total += s.frame;
                s.max = s.max.max(s.frame);
                s.frame = Duration::ZERO;
            }
            let now = Instant::now();
            if now - p.window_start >= WINDOW && p.frames > 0 {
                log::info!("{}", summary(p, now));
                p.window_start = now;
                p.frames = 0;
                p.interval_total = Duration::ZERO;
                p.interval_max = Duration::ZERO;
                p.cpu_total = Duration::ZERO;
                p.cpu_max = Duration::ZERO;
                for s in &mut p.segments {
                    s.total = Duration::ZERO;
                    s.max = Duration::ZERO;
                }
            }
        });
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    /// The per-frame line of `WTE_FRAME_LOG`: this frame's interval, its UI CPU time and every segment that ran in it.
    fn frame_line(interval: Duration, cpu: Duration, segments: &[Segment]) -> String {
        let parts: Vec<String> =
            segments.iter().filter(|s| s.frame > Duration::ZERO).map(|s| format!("{} {:.2}", s.name, ms(s.frame))).collect();
        format!("frame timing: interval {:.2} ms, ui cpu {:.2} ms; segments ms: {}", ms(interval), ms(cpu), parts.join(", "))
    }

    fn summary(p: &Profile, now: Instant) -> String {
        let n = p.frames.max(1) as f64;
        let secs = (now - p.window_start).as_secs_f64().max(1e-9);
        let intervals = (p.frames.saturating_sub(1)).max(1) as f64;
        let mut segments: Vec<&Segment> = p.segments.iter().filter(|s| s.total > Duration::ZERO).collect();
        segments.sort_by_key(|s| std::cmp::Reverse(s.total));
        let parts: Vec<String> =
            segments.iter().map(|s| format!("{} {:.2}/{:.1}", s.name, ms(s.total) / n, ms(s.max))).collect();
        format!(
            "frame profile: {} frames, {:.1} fps, interval avg {:.2} ms max {:.1} ms, ui cpu avg {:.2} ms max {:.1} ms; \
             segments avg/max ms: {}",
            p.frames,
            p.frames as f64 / secs,
            ms(p.interval_total) / intervals,
            ms(p.interval_max),
            ms(p.cpu_total) / n,
            ms(p.cpu_max),
            parts.join(", ")
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn segments_accumulate_per_frame_and_the_off_state_is_a_no_op() {
            // Off: nothing is recorded.
            std::thread::spawn(|| {
                assert!(!enabled());
                drop(scope("x"));
                frame_begin();
                frame_end();
                PROFILE.with(|p| assert!(p.borrow().is_none()));
            })
            .join()
            .unwrap();
            std::thread::spawn(|| {
                enable();
                frame_begin();
                drop(scope("a"));
                drop(scope("a"));
                drop(scope("b"));
                frame_end();
                PROFILE.with(|p| {
                    let p = p.borrow();
                    let p = p.as_ref().unwrap();
                    assert_eq!(p.frames, 1);
                    let names: Vec<_> = p.segments.iter().map(|s| s.name).collect();
                    assert_eq!(names, ["a", "b"]);
                    assert!(p.segments.iter().all(|s| s.frame == Duration::ZERO));
                    assert!(summary(p, Instant::now()).contains("frame profile: 1 frames"));
                });
            })
            .join()
            .unwrap();
        }

        /// Gate stage 3: the per-frame line of `WTE_FRAME_LOG` names the interval, the CPU time and only the segments
        /// that ran in the frame.
        #[test]
        fn the_per_frame_line_lists_the_segments_of_one_frame() {
            let segments = [
                Segment { name: "side panel", frame: Duration::from_micros(1250), ..Default::default() },
                Segment { name: "dispatch", ..Default::default() },
                Segment { name: "central", frame: Duration::from_millis(3), ..Default::default() },
            ];
            let line = frame_line(Duration::from_micros(16_700), Duration::from_micros(4_500), &segments);
            assert_eq!(line, "frame timing: interval 16.70 ms, ui cpu 4.50 ms; segments ms: side panel 1.25, central 3.00");
        }
    }
}
