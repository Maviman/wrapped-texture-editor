// Owner: app-shell (A1). Entry point of ARCHITECTURE.md 8.3 (R6): logging to a file, the panic hook, the
// startup-failure message box, `NativeOptions`, `eframe::run_native` and the process exit code.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

use eframe::{egui, egui_wgpu, wgpu};

mod action;
mod app;
mod brush_flows;
mod cli;
mod companion;
#[cfg(test)]
mod control_sweep;
mod crash;
mod dialogs;
mod dispatch;
mod dnd;
mod gpu;
mod gpu_check;
mod io_flows;
mod jobs;
mod link;
mod object_ops;
mod ole;
mod panels;
mod pen;
mod prefs;
mod selftest;
mod shortcuts;
mod stamp_bakes;
mod state;
mod sticker;
mod text_ops;
mod theme;
mod toasts;
mod tools;
mod view;

/// Exit code of a startup failure (bad arguments, no wgpu adapter, window creation failure), section 8.11.
const EXIT_STARTUP_FAILURE: i32 = 4;
/// Exit code after a panic on any thread.
const EXIT_PANIC: i32 = 101;

/// The log file in use (shown in the error box), set once at startup.
static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
/// True in self-test mode: no message boxes.
static SELFTEST: AtomicBool = AtomicBool::new(false);

fn main() {
    // `--gpu-check REPORT`: list the graphics adapters into REPORT and exit (8.11). No window, no log file, no message box:
    // the installer and the release CI run it in the background.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if raw.first().is_some_and(|a| a == "--gpu-check") {
        let code = match raw.get(1).filter(|p| !p.is_empty()) {
            Some(report) if raw.len() == 2 => gpu_check::run(Path::new(report)),
            _ => gpu_check::EXIT_ERROR,
        };
        std::process::exit(code);
    }
    // `args_os`: `std::env::args` panics on a file name that is not valid Unicode (8.11).
    let parsed = cli::parse(std::env::args_os().skip(1));
    // Also when the arguments do not parse: an automated run must never block on a message box.
    let selftest = std::env::args_os().skip(1).any(|a| a == "--screenshot");
    SELFTEST.store(selftest, Ordering::SeqCst);

    // 1. Log file: --log, else <OUT>.log in self-test mode, else %APPDATA%\WrappedTextureEditor\log.txt.
    let log_path = match &parsed {
        Ok(args) => args.log.clone().or_else(|| args.out_stem().map(|s| with_suffix(&s, ".log"))),
        // Unparsable arguments: still log next to the screenshot when one was named.
        Err(_) => {
            let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
            raw.windows(2)
                .find(|w| w[0] == "--screenshot" && !w[1].is_empty())
                .map(|w| with_suffix(&PathBuf::from(&w[1]).with_extension(""), ".log"))
        }
    }
    .or_else(|| prefs::app_data_dir().map(|d| d.join("log.txt")));
    let (log_path, log_handle) = log_path.and_then(|p| init_logging(&p)).unzip();
    let _ = LOG_PATH.set(log_path.clone());
    log::info!("Wrapped Texture Editor {} starting", env!("CARGO_PKG_VERSION"));
    // Native crashes (access violations in drivers or injected DLLs, heap corruption) never reach the panic hook.
    crash::install(log_handle.flatten(), selftest);

    // 2. Panic hook for every thread.
    install_panic_hook();

    let args = match parsed {
        Ok(args) => args,
        Err(error) => fail_startup(&format!("Invalid command line: {error}")),
    };
    log::info!("arguments: {args:?}");
    let exit_code = Arc::new(AtomicI32::new(0));

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(io_flows::APP_TITLE)
        .with_icon(theme::app_icon())
        .with_drag_and_drop(true);
    viewport = if selftest {
        // A self-test run opens in the background: shown without taking the focus, outside every monitor and without
        // a taskbar button, so a batch of runs never covers or pulls the user out of what they are doing. The window
        // still renders and captures (only a hidden or minimized window stops drawing; the capture reads the GPU
        // frame, and the runner fixes the zoom at 1 physical pixel per point), and `selftest::Runner::raw_input_hook`
        // reports the window as focused, so a run gives the same result as in the foreground.
        let size = [args.size[0] as f32, args.size[1] as f32];
        viewport
            .with_inner_size(size)
            .with_position(offscreen_position(size))
            .with_active(false)
            .with_taskbar(false)
    } else {
        // The remembered window size (default 1280x800), never below the minimum, and the maximized state. The first
        // frame fits a window that is larger than its monitor (`app::fit_to_monitor`).
        let prefs = prefs::load();
        let [w, h] = prefs.window_size;
        viewport
            .with_inner_size([w.max(900.0), h.max(600.0)])
            .with_min_inner_size([900.0, 600.0])
            .with_maximized(prefs.window_maximized)
    };
    let defaults = eframe::NativeOptions::default();
    let options = eframe::NativeOptions {
        viewport,
        multisampling: 0,
        depth_buffer: 0,
        persist_window: false,
        system_font_fallback: false,
        dithering: defaults.dithering && !selftest,
        wgpu_options: wgpu_options(defaults.wgpu_options.clone()),
        ..defaults
    };

    let app_exit_code = Arc::clone(&exit_code);
    let result = eframe::run_native(
        io_flows::APP_TITLE,
        options,
        Box::new(move |cc| match app::WteApp::new(cc, args, app_exit_code) {
            Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            Err(error) => Err(format!("{error:#}").into()),
        }),
    );

    match result {
        Ok(()) => {
            let code = exit_code.load(Ordering::SeqCst);
            log::info!("exit code {code}");
            log::logger().flush();
            std::process::exit(code);
        }
        Err(error) => fail_startup(&format!("Wrapped Texture Editor could not start: {error}")),
    }
}

/// The wgpu setup (8.1, 8.4): the backends of `WGPU_BACKEND` (else all primary ones plus GL) as egui-wgpu chooses them,
/// but
/// * the adapter is picked by `adapter_rank`: Vulkan first. DX12 is the fallback (with MSAA off, `gpu::msaa_for`), GL
///   the last resort;
/// * the device gets the adapter's own `max_texture_dimension_2d` and `max_buffer_size` instead of egui-wgpu's 8192 /
///   wgpu's default 256 MiB: a window wider than 8192 px (several monitors) and large meshes stay within the limits.
///   The window is additionally capped at the surface limit (`WteApp::logic`) and meshes are checked
///   (`gpu::scene::check_mesh_fits`), so neither ends in a wgpu validation panic.
fn wgpu_options(mut options: egui_wgpu::WgpuConfiguration) -> egui_wgpu::WgpuConfiguration {
    if let egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_setup {
        create.native_adapter_selector = Some(Arc::new(|adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
            let usable = adapters.iter().filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)));
            let best = usable.min_by_key(|a| {
                let info = a.get_info();
                gpu_check::adapter_rank(info.backend, info.device_type)
            }).cloned();
            for a in adapters {
                let info = a.get_info();
                log::info!("graphics adapter available: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
            }
            best.ok_or_else(|| "no graphics adapter can draw into the window".to_owned())
        }));
        create.device_descriptor = Arc::new(|adapter: &wgpu::Adapter| {
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };
            wgpu::DeviceDescriptor {
                label: Some("wte device"),
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: supported.max_texture_dimension_2d.max(base.max_texture_dimension_2d),
                    max_buffer_size: supported.max_buffer_size.max(base.max_buffer_size),
                    ..base
                },
                ..Default::default()
            }
        });
    }
    options
}

/// The outer position (points) of a self-test window: up and to the left of the whole virtual screen (all monitors),
/// with a margin larger than the window and its frame, so no part of it shows on any monitor. egui-winit passes it as
/// a logical position; a scale factor of 1 or more only moves it further out.
fn offscreen_position(size: [f32; 2]) -> [f32; 2] {
    let (left, top) = virtual_screen_origin();
    [left as f32 - 2.0 * size[0] - 1000.0, top as f32 - 2.0 * size[1] - 1000.0]
}

/// Top-left corner of the virtual screen in physical pixels (negative when a monitor lies left of or above the
/// primary one).
#[cfg(windows)]
fn virtual_screen_origin() -> (i32, i32) {
    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    // SAFETY: GetSystemMetrics takes an integer and reads no memory of ours.
    unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN)) }
}

#[cfg(not(windows))]
fn virtual_screen_origin() -> (i32, i32) {
    (0, 0)
}

/// `path` + `suffix` (e.g. `E:\out\shot` + `.log`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Opens a log file for this process only: created or truncated, and shared for reading but not for writing or
/// deleting, so a second instance cannot truncate the log of a running one (it gets a sharing violation instead).
fn open_exclusive_log(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// `log.txt` -> `log-<pid>.txt`: the log of an instance started while another one holds `log.txt`.
fn per_process_log(path: &Path) -> PathBuf {
    let stem = path.file_stem().map_or_else(|| "log".into(), |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map_or_else(String::new, |e| format!(".{}", e.to_string_lossy()));
    path.with_file_name(format!("{stem}-{}{ext}", std::process::id()))
}

/// Deletes all but the newest `keep` per-process logs (`<stem>-<pid>.<ext>`) next to `path` (the main log itself is
/// never touched; a file still held by its instance cannot be deleted and stays).
fn prune_process_logs(path: &Path, keep: usize) {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().map(|s| s.to_string_lossy().into_owned())) else { return };
    let Ok(entries) = std::fs::read_dir(if dir.as_os_str().is_empty() { Path::new(".") } else { dir }) else { return };
    let prefix = format!("{stem}-");
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix(&prefix).is_some_and(|rest| rest.split('.').next().is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())))
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    logs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, old) in logs.into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

/// Logs to `path` (directory created, file truncated); when another instance holds `path`, to `log-<pid>.txt` next to
/// it. Returns the file in use and a second handle on it for the crash recorder, `None` when no file could be created.
/// Every record also goes into the crash recorder's ring of recent lines (`crash::Tee`).
fn init_logging(path: &Path) -> Option<(PathBuf, Option<std::fs::File>)> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    let (file, used) = match open_exclusive_log(path) {
        Ok(file) => (file, path.to_path_buf()),
        Err(_) => {
            let own = per_process_log(path);
            prune_process_logs(path, 4);
            (open_exclusive_log(&own).ok()?, own)
        }
    };
    let ok = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        // wgpu and naga are chatty at Info level.
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal", log::LevelFilter::Warn)
        .filter_module("naga", log::LevelFilter::Warn)
        .target(env_logger::Target::Pipe(Box::new(crash::Tee(file.try_clone().ok()?))))
        .try_init()
        .is_ok();
    ok.then_some((used, Some(file)))
}

/// Name of the thread that shows the fatal-error box.
const ERROR_BOX_THREAD: &str = "error box";
/// Set by the first fatal error (panic or lost device); later ones do not show a second box.
static FATAL: AtomicBool = AtomicBool::new(false);

/// Logs a panic (message and location), shows the error box unless in self-test mode, and exits with code 101.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Debug builds still print to the console.
        previous(info);
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_owned());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        log::error!("panic in thread '{thread}' at {location}: {message}");
        log::logger().flush();
        fatal_exit(&format!("The application stopped because of an internal error:\n{message}\n({location})"), EXIT_PANIC);
    }));
}

/// Ends the process after a fatal error: the error box (not in self-test mode), then `std::process::exit(code)`.
///
/// The box runs on a fresh thread that this thread waits for without pumping messages. A panic usually happens on
/// the UI thread inside a window-message callback of winit; a modal message loop there runs re-entrantly in winit's
/// window procedure, the main window's paint messages never stop arriving, and the box stays invisible while the
/// process spins at full CPU and never exits. On its own thread with no owner window the box has its own message
/// queue. Only the first fatal error shows a box; a later one (another thread) waits for that box to end the
/// process. The process always exits, also when the box cannot be shown.
pub(crate) fn fatal_exit(text: &str, code: i32) -> ! {
    let on_box_thread = std::thread::current().name() == Some(ERROR_BOX_THREAD);
    if SELFTEST.load(Ordering::SeqCst) || on_box_thread {
        std::process::exit(code);
    }
    if FATAL.swap(true, Ordering::SeqCst) {
        // Another thread is showing the box and will exit the process.
        loop {
            std::thread::park();
        }
    }
    let text = text.to_owned();
    match std::thread::Builder::new().name(ERROR_BOX_THREAD.to_owned()).spawn(move || error_box(&text)) {
        Ok(handle) => {
            let _ = handle.join();
        }
        Err(e) => log::error!("cannot show the error box: {e}"),
    }
    log::logger().flush();
    std::process::exit(code);
}

/// Logs a startup failure, shows the error box unless in self-test mode and exits with code 4.
fn fail_startup(error: &str) -> ! {
    log::error!("{error}");
    log::logger().flush();
    eprintln!("{error}");
    if !SELFTEST.load(Ordering::SeqCst) {
        error_box(error);
    }
    std::process::exit(EXIT_STARTUP_FAILURE);
}

/// A native message box that needs no window.
fn error_box(error: &str) {
    let log = match LOG_PATH.get() {
        Some(Some(p)) => p.display().to_string(),
        _ => "(no log file)".to_owned(),
    };
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(io_flows::APP_TITLE)
        .set_description(format!("{error}\n\nLog file: {log}"))
        .show();
}
