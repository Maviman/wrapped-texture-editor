// Owner: app-shell (A1). The `--gpu-check REPORT` mode of ARCHITECTURE.md 8.11: lists the graphics adapters wgpu can use,
// without a window, and says whether the app can draw. The installer runs it before and after installing (hidden, no
// focus change) and the release CI runs it on the build machine.

use std::fmt::Write as _;
use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use eframe::wgpu;

/// A hardware Vulkan or DirectX 12 adapter is present: the app runs normally.
pub const EXIT_OK: i32 = 0;
/// Only software or OpenGL adapters: the app starts but may be slow or limited.
pub const EXIT_LIMITED: i32 = 2;
/// No graphics adapter at all: the app cannot start.
pub const EXIT_NONE: i32 = 3;
/// The check itself failed (bad arguments, report not writable, a panic in the driver).
pub const EXIT_ERROR: i32 = 5;

/// What the check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Limited,
    None,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Limited => "limited",
            Verdict::None => "none",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Ok => EXIT_OK,
            Verdict::Limited => EXIT_LIMITED,
            Verdict::None => EXIT_NONE,
        }
    }
}

/// One adapter as the report shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterLine {
    pub name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    pub driver: String,
}

impl AdapterLine {
    /// A hardware adapter on one of the app's main backends (Vulkan, DirectX 12).
    pub fn is_full(&self) -> bool {
        matches!(self.backend, wgpu::Backend::Vulkan | wgpu::Backend::Dx12) && self.device_type != wgpu::DeviceType::Cpu
    }
}

/// Rank of an adapter, lower is better: software adapters last, then Vulkan before DX12 before Metal before GL, then
/// discrete before integrated before virtual GPUs. The app's adapter selector (`main.rs`) and the report's `best=` use
/// it, so the installer names the adapter the app will really use.
pub fn adapter_rank(backend: wgpu::Backend, device_type: wgpu::DeviceType) -> (u8, u8, u8) {
    let software = u8::from(device_type == wgpu::DeviceType::Cpu);
    let backend = match backend {
        wgpu::Backend::Vulkan => 0,
        wgpu::Backend::Dx12 => 1,
        wgpu::Backend::Metal => 2,
        wgpu::Backend::Gl => 3,
        _ => 4,
    };
    let device = match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    (software, backend, device)
}

/// `Ok` with a hardware Vulkan or DX12 adapter, `Limited` with only software or OpenGL adapters, `None` without any.
pub fn classify(adapters: &[AdapterLine]) -> Verdict {
    if adapters.iter().any(AdapterLine::is_full) {
        Verdict::Ok
    } else if adapters.is_empty() {
        Verdict::None
    } else {
        Verdict::Limited
    }
}

/// The plain-text report: `key=value` lines, easy to read from Inno Setup's Pascal script and PowerShell.
pub fn report_text(adapters: &[AdapterLine], verdict: Verdict) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "app_version={}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "result={}", verdict.word());
    let _ = writeln!(out, "vulkan={}", if adapters.iter().any(|a| a.backend == wgpu::Backend::Vulkan) { "yes" } else { "no" });
    let _ = writeln!(out, "dx12={}", if adapters.iter().any(|a| a.backend == wgpu::Backend::Dx12) { "yes" } else { "no" });
    let best = adapters.iter().min_by_key(|a| adapter_rank(a.backend, a.device_type));
    let _ = writeln!(out, "best={}", best.map_or_else(String::new, |a| format!("{} ({:?})", a.name, a.backend)));
    for a in adapters {
        let driver = if a.driver.is_empty() { String::new() } else { format!(", driver {}", a.driver) };
        let _ = writeln!(out, "adapter={} ({:?}, {:?}{driver})", a.name, a.backend, a.device_type);
    }
    out
}

/// Minimal executor: wgpu's native futures resolve without an external reactor.
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// The adapters wgpu offers on the backends of `WGPU_BACKEND` (else Vulkan, DirectX 12 and OpenGL), deduplicated.
fn list_adapters() -> Vec<AdapterLine> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    if std::env::var_os("WGPU_BACKEND").is_none() {
        desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::GL;
    }
    let backends = desc.backends;
    let instance = wgpu::Instance::new(desc);
    let mut lines: Vec<AdapterLine> = block_on(instance.enumerate_adapters(backends))
        .iter()
        .map(|a| {
            let info = a.get_info();
            // Vulkan: driver name + version; DX12 leaves driver_info empty and puts the version in `driver`.
            let driver = [info.driver.as_str(), info.driver_info.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
            AdapterLine { name: info.name, backend: info.backend, device_type: info.device_type, driver }
        })
        .collect();
    lines.dedup();
    lines
}

/// How long listing the adapters may take before the check gives up (a hung driver must not freeze the installer).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Runs the check and writes the report to `report`; returns the process exit code. The caller exits the process
/// right after, which also ends a listing thread that is still stuck in a driver.
pub fn run(report: &Path) -> i32 {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::panic::catch_unwind(list_adapters).map_err(|_| "the graphics driver failed while listing adapters"));
    });
    let received = rx.recv_timeout(TIMEOUT);
    let timed_out = received.is_err();
    let found = received.unwrap_or(Err("listing the graphics adapters did not finish within 20 s"));
    let text = match &found {
        Ok(adapters) => report_text(adapters, classify(adapters)),
        Err(error) => format!("app_version={}\nresult=error\nerror={error}\n", env!("CARGO_PKG_VERSION")),
    };
    let written = std::fs::write(report, text).is_ok();
    if timed_out {
        // The listing thread is stuck in a driver, possibly holding the loader lock: a normal exit would run every DLL's
        // detach code and could hang on it. Terminate without that.
        terminate_now(EXIT_ERROR);
    }
    if !written {
        return EXIT_ERROR;
    }
    match found {
        Ok(adapters) => classify(&adapters).exit_code(),
        Err(_) => EXIT_ERROR,
    }
}

/// Ends the process at once, without running DLL detach code (`TerminateProcess` on the current process).
#[cfg(windows)]
fn terminate_now(code: i32) -> ! {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn TerminateProcess(process: *mut std::ffi::c_void, exit_code: u32) -> i32;
    }
    // SAFETY: the pseudo handle of the current process is always valid; TerminateProcess on it does not return.
    unsafe {
        TerminateProcess(GetCurrentProcess(), code as u32);
    }
    std::process::exit(code)
}

#[cfg(not(windows))]
fn terminate_now(code: i32) -> ! {
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(name: &str, backend: wgpu::Backend, device_type: wgpu::DeviceType) -> AdapterLine {
        AdapterLine { name: name.to_owned(), backend, device_type, driver: String::new() }
    }

    #[test]
    fn a_hardware_vulkan_or_dx12_adapter_is_ok() {
        let gpu = line("RTX", wgpu::Backend::Vulkan, wgpu::DeviceType::DiscreteGpu);
        let warp = line("Microsoft Basic Render Driver", wgpu::Backend::Dx12, wgpu::DeviceType::Cpu);
        assert_eq!(classify(&[warp.clone(), gpu]), Verdict::Ok);
        assert_eq!(classify(&[line("Intel", wgpu::Backend::Dx12, wgpu::DeviceType::IntegratedGpu)]), Verdict::Ok);
        assert_eq!(classify(&[warp]), Verdict::Limited, "a software adapter only is limited");
        assert_eq!(classify(&[line("GDI", wgpu::Backend::Gl, wgpu::DeviceType::Other)]), Verdict::Limited);
        assert_eq!(classify(&[]), Verdict::None);
        assert_eq!([Verdict::Ok, Verdict::Limited, Verdict::None].map(Verdict::exit_code), [EXIT_OK, EXIT_LIMITED, EXIT_NONE]);
    }

    #[test]
    fn the_report_names_the_result_the_backends_and_the_best_adapter() {
        let warp = line("WARP", wgpu::Backend::Dx12, wgpu::DeviceType::Cpu);
        let mut gpu = line("RTX 5070", wgpu::Backend::Vulkan, wgpu::DeviceType::DiscreteGpu);
        gpu.driver = "591.44".to_owned();
        let text = report_text(&[warp, gpu.clone()], Verdict::Ok);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "result=ok");
        assert_eq!(lines[2], "vulkan=yes");
        assert_eq!(lines[3], "dx12=yes");
        assert_eq!(lines[4], "best=RTX 5070 (Vulkan)", "the hardware adapter, not the first one");
        assert!(lines.contains(&"adapter=RTX 5070 (Vulkan, DiscreteGpu, driver 591.44)"), "{text}");
        let igpu = line("Intel UHD", wgpu::Backend::Vulkan, wgpu::DeviceType::IntegratedGpu);
        let dgpu = line("RTX 4060 Laptop", wgpu::Backend::Vulkan, wgpu::DeviceType::DiscreteGpu);
        let hybrid = report_text(&[igpu, dgpu], Verdict::Ok);
        assert!(hybrid.contains("best=RTX 4060 Laptop (Vulkan)"), "the adapter the app picks, not the first listed: {hybrid}");
        let none = report_text(&[], Verdict::None);
        assert!(none.contains("result=none\nvulkan=no\ndx12=no\nbest=\n"), "{none}");
    }
}
