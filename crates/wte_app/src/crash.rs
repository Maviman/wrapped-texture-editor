// Owner: app-shell (A1). Native crash recorder of ARCHITECTURE.md 8.3: what the panic hook cannot see (access
// violations, heap corruption, stack overflow, illegal instructions inside native code such as a graphics driver or an
// injected overlay DLL) still leaves a report, a minidump and a log line. Also the startup diagnostics (graphics
// environment variables, loaded hook / overlay modules) and the WTE_DEBUG_NATIVE_CRASH test hook.
//
// Everything that runs while crashing avoids the heap where it can: text is formatted into fixed stack buffers and
// written with WriteFile to handles opened before the crash (the log) or with CreateFileW on preformatted UTF-16 paths.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;

// ---------------------------------------------------------------------------------------------------------------------
// Ring buffer of the last app actions and log lines (copied into the crash report).

const RING_SLOTS: usize = 24;
const RING_LINE: usize = 200;

struct Slot {
    len: AtomicU16,
    bytes: [AtomicU8; RING_LINE],
}

static RING: [Slot; RING_SLOTS] = [const { Slot { len: AtomicU16::new(0), bytes: [const { AtomicU8::new(0) }; RING_LINE] } }; RING_SLOTS];
/// Number of lines ever recorded; the next line goes into slot `RING_NEXT % RING_SLOTS`.
static RING_NEXT: AtomicUsize = AtomicUsize::new(0);
static START: OnceLock<Instant> = OnceLock::new();

/// Records one line (truncated to 200 bytes, newlines removed) in the ring. Allocation-free.
pub fn note(line: &[u8]) {
    let line = trim_ascii_end(line);
    if line.is_empty() {
        return;
    }
    let slot = &RING[RING_NEXT.fetch_add(1, Ordering::Relaxed) % RING_SLOTS];
    let n = cut_len(line, RING_LINE);
    slot.len.store(0, Ordering::Relaxed);
    for (i, b) in line[..n].iter().enumerate() {
        slot.bytes[i].store(if *b == b'\r' || *b == b'\n' { b' ' } else { *b }, Ordering::Relaxed);
    }
    slot.len.store(n as u16, Ordering::Release);
}

/// Records a dispatched app action (the log has them only at debug level) in the ring.
pub fn note_action(action: &dyn std::fmt::Debug) {
    let secs = START.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    let mut buf = StackBuf::<RING_LINE>::new();
    let _ = std::fmt::Write::write_fmt(&mut buf, format_args!("[{secs:9.3}s] action {action:?}"));
    note(buf.as_bytes());
}

/// The longest prefix of `line` of at most `max` bytes that ends at a UTF-8 character boundary.
fn cut_len(line: &[u8], max: usize) -> usize {
    let mut n = line.len().min(max);
    while n > 0 && n < line.len() && (line[n] & 0xC0) == 0x80 {
        n -= 1;
    }
    n
}

fn trim_ascii_end(mut s: &[u8]) -> &[u8] {
    while let [rest @ .., last] = s {
        if last.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

/// The log writer: writes every record to the log file and copies its lines into the ring.
pub struct Tee(pub std::fs::File);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_all(buf)?;
        for line in buf.split(|b| *b == b'\n') {
            note(line);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Fixed-size formatting buffer.

struct StackBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackBuf<N> {
    fn new() -> Self {
        StackBuf { buf: [0; N], len: 0 }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
    fn push(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }
    /// Appends UTF-16 text (a module path) as UTF-8.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn push_wide(&mut self, wide: &[u16]) {
        for c in char::decode_utf16(wide.iter().copied()) {
            let mut tmp = [0u8; 4];
            self.push(c.unwrap_or(char::REPLACEMENT_CHARACTER).encode_utf8(&mut tmp).as_bytes());
        }
    }
}

impl<const N: usize> std::fmt::Write for StackBuf<N> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Installation.

/// Paths prepared at startup, used while crashing.
struct Setup {
    /// A handle on the log file (shares the file pointer with the logger's handle).
    log: Option<std::fs::File>,
    /// `<exe dir>\crash` as UTF-16 without a terminator. No fallback elsewhere (the log dir is on the system drive): when
    /// it cannot be written, the full report is still in the log.
    dirs: Vec<Vec<u16>>,
    selftest: bool,
}

static SETUP: OnceLock<Setup> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The directory crash reports are written to (first choice).
pub fn crash_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|d| d.join("crash"))
}

/// Installs the native crash recorder: call right after logging is set up. `log` is a second handle on the log file.
pub fn install(log: Option<std::fs::File>, selftest: bool) {
    let _ = START.set(Instant::now());
    let dirs: Vec<PathBuf> = crash_dir().into_iter().collect();
    #[cfg(windows)]
    let dirs: Vec<Vec<u16>> = {
        use std::os::windows::ffi::OsStrExt;
        dirs.iter().map(|d| d.as_os_str().encode_wide().collect()).collect()
    };
    #[cfg(not(windows))]
    let dirs: Vec<Vec<u16>> = dirs.iter().map(|d| d.to_string_lossy().encode_utf16().collect()).collect();
    let _ = SETUP.set(Setup { log, dirs, selftest });
    #[cfg(windows)]
    win::install();
    INSTALLED.store(true, Ordering::SeqCst);
    log::info!("native crash recorder installed (reports go to {})", crash_dir().map_or_else(|| "?".to_owned(), |d| d.display().to_string()));
}

/// Installs the unhandled-exception filter again (a graphics driver or an injected DLL may have replaced it during GPU
/// initialization) and logs when it had been replaced.
pub fn reassert_filter() {
    #[cfg(windows)]
    if INSTALLED.load(Ordering::SeqCst) {
        win::reassert_filter();
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Startup diagnostics.

/// Environment variables that change the graphics backend or the Vulkan layers.
const GRAPHICS_ENV: &[&str] = &[
    "WGPU_BACKEND",
    "WGPU_ADAPTER_NAME",
    "WGPU_POWER_PREF",
    "VK_LOADER_LAYERS_DISABLE",
    "VK_LOADER_LAYERS_ENABLE",
    "VK_INSTANCE_LAYERS",
    "DISABLE_VULKAN_OBS_CAPTURE",
    "WTE_DEBUG_NATIVE_CRASH",
];

/// Parts of module paths that mark hooks and overlays injected into the process.
const HOOK_PATTERNS: &[&str] = &[
    "obs", "graphics-hook", "overlay", "steam", "fossilize", "rtss", "rivatuner", "nvidia overlay", "nvspcap", "discord", "medal",
    "amdihk", "reshade", "afterburner",
];
/// Parts of module paths of graphics runtimes and drivers (logged too: they tell which driver code is in the process).
const GRAPHICS_PATTERNS: &[&str] =
    &["vulkan-1", "amdvlk", "amdxc", "atiumd", "nvoglv", "nvwgf", "nvldumd", "igvk", "igd", "d3d12", "dxgi", "dxil", "dxcompiler"];

/// Logs the graphics environment variables and the hook / overlay / graphics driver modules loaded in the process.
pub fn log_startup_diagnostics(adapter: &str) {
    log::info!("graphics backend in use: {adapter}");
    for name in GRAPHICS_ENV {
        if let Some(value) = std::env::var_os(name) {
            log::info!("environment: {name}={}", value.to_string_lossy());
        }
    }
    let modules = loaded_modules();
    let mut hooks = 0;
    for m in &modules {
        let lower = m.to_lowercase();
        let file = lower.rsplit(['\\', '/']).next().unwrap_or(&lower);
        if HOOK_PATTERNS.iter().any(|p| lower.contains(p)) {
            hooks += 1;
            log::warn!("hook/overlay module loaded: {m}");
        } else if GRAPHICS_PATTERNS.iter().any(|p| file.contains(p)) {
            log::info!("graphics module loaded: {m}");
        }
    }
    log::info!("{} modules loaded, {hooks} of them hooks/overlays", modules.len());
}

/// Full paths of the modules loaded in this process.
fn loaded_modules() -> Vec<String> {
    #[cfg(windows)]
    {
        win::loaded_modules()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Test hook.

/// `WTE_DEBUG_NATIVE_CRASH=av|heap|stack` crashes the process on purpose (an access violation, heap corruption, a
/// stack overflow) to test the crash recorder, also in release builds. Does nothing when the variable is not set.
pub fn debug_trigger_from_env() {
    let Ok(kind) = std::env::var("WTE_DEBUG_NATIVE_CRASH") else { return };
    let kind = kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "av" | "heap" | "stack") {
        log::warn!("WTE_DEBUG_NATIVE_CRASH={kind} is not one of av, heap, stack: ignored");
        return;
    }
    log::warn!("WTE_DEBUG_NATIVE_CRASH={kind}: crashing on purpose to test the crash recorder");
    log::logger().flush();
    #[cfg(windows)]
    match kind.as_str() {
        "av" => win::trigger_access_violation(),
        "heap" => win::trigger_heap_corruption(),
        _ => {
            let deep = std::hint::black_box(0u64);
            log::info!("unreachable: {}", recurse(deep));
        }
    }
}

#[cfg(windows)]
#[inline(never)]
fn recurse(n: u64) -> u64 {
    let frame = std::hint::black_box([n; 512]);
    if std::hint::black_box(true) { recurse(frame[7].wrapping_add(1)).wrapping_add(frame[3]) } else { frame[0] }
}

// ---------------------------------------------------------------------------------------------------------------------
// Windows implementation.

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;
    type Bool = i32;
    type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;
    type ThreadStart = unsafe extern "system" fn(*mut c_void) -> u32;

    #[repr(C)]
    pub struct ExceptionRecord {
        pub code: u32,
        pub flags: u32,
        pub record: *mut ExceptionRecord,
        pub address: *mut c_void,
        pub number_parameters: u32,
        pub information: [usize; 15],
    }

    #[repr(C)]
    pub struct ExceptionPointers {
        pub record: *mut ExceptionRecord,
        pub context: *mut c_void,
    }

    /// `MINIDUMP_EXCEPTION_INFORMATION` (dbghelp.h packs it to 4 bytes: size 16 on x64).
    #[repr(C, packed(4))]
    struct MinidumpExceptionInformation {
        thread_id: u32,
        exception_pointers: *mut ExceptionPointers,
        client_pointers: Bool,
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
        fn AddVectoredExceptionHandler(first: u32, handler: Filter) -> *mut c_void;
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut Handle) -> Bool;
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn GetModuleFileNameW(module: Handle, filename: *mut u16, size: u32) -> u32;
        fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> u32;
        fn GetCurrentThreadId() -> u32;
        fn GetCurrentThread() -> Handle;
        fn TerminateProcess(process: Handle, exit_code: u32) -> Bool;
        fn CreateDirectoryW(path: *const u16, security: *mut c_void) -> Bool;
        fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
        fn WriteFile(file: Handle, buffer: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> Bool;
        fn FlushFileBuffers(file: Handle) -> Bool;
        fn CloseHandle(handle: Handle) -> Bool;
        fn GetLastError() -> u32;
        fn CreateThread(security: *mut c_void, stack: usize, start: ThreadStart, param: *mut c_void, flags: u32, id: *mut u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
        fn Sleep(ms: u32);
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
        fn GetProcessHeap() -> Handle;
        fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut c_void;
        fn HeapFree(heap: Handle, flags: u32, mem: *mut c_void) -> Bool;
        fn K32EnumProcessModules(process: Handle, modules: *mut Handle, cb: u32, needed: *mut u32) -> Bool;
    }

    #[link(name = "dbghelp", kind = "raw-dylib")]
    unsafe extern "system" {
        fn MiniDumpWriteDump(
            process: Handle,
            process_id: u32,
            file: Handle,
            dump_type: u32,
            exception: *const MinidumpExceptionInformation,
            user_stream: *const c_void,
            callback: *const c_void,
        ) -> Bool;
    }

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn MessageBoxW(owner: Handle, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const CREATE_ALWAYS: u32 = 2;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;
    const INFINITE: u32 = 0xFFFF_FFFF;
    /// How long the crash message box may stay open before the process ends anyway. Bounded because after heap
    /// corruption the box thread can block forever on the heap lock held by the crashed thread, which would leave a
    /// frozen window instead of an exit (the report is written before the box, so nothing is lost).
    const BOX_WAIT_MS: u32 = 120_000;
    /// How long the stack-overflow reporting thread may take (report, dump and box).
    const OVERFLOW_WAIT_MS: u32 = BOX_WAIT_MS + 60_000;
    const MB_ICONERROR: u32 = 0x10;
    const MB_TOPMOST: u32 = 0x4_0000;
    /// MiniDumpWithUnloadedModules | MiniDumpWithIndirectlyReferencedMemory | MiniDumpWithThreadInfo.
    const DUMP_TYPE: u32 = 0x20 | 0x40 | 0x1000;

    const ACCESS_VIOLATION: u32 = 0xC000_0005;
    const ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
    const STACK_OVERFLOW: u32 = 0xC000_00FD;
    const HEAP_CORRUPTION: u32 = 0xC000_0374;
    const STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;

    fn code_name(code: u32) -> &'static str {
        match code {
            ACCESS_VIOLATION => "access violation",
            ILLEGAL_INSTRUCTION => "illegal instruction",
            STACK_OVERFLOW => "stack overflow",
            HEAP_CORRUPTION => "heap corruption",
            STACK_BUFFER_OVERRUN => "stack buffer overrun / fail-fast",
            0xC000_0094 => "integer division by zero",
            0xC000_0096 => "privileged instruction",
            0xC000_0006 => "in-page error",
            0x8000_0003 => "breakpoint",
            _ => "native exception",
        }
    }

    pub fn install() {
        // SAFETY: plain Win32 calls with valid function pointers.
        unsafe {
            AddVectoredExceptionHandler(1, vectored_handler);
            SetUnhandledExceptionFilter(Some(unhandled_filter));
        }
    }

    pub fn reassert_filter() {
        // SAFETY: as in `install`.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(unhandled_filter)) };
        if previous.is_some_and(|p| p as usize != unhandled_filter as Filter as usize) {
            log::warn!("another module had replaced the crash filter during graphics initialization; reinstalled");
        }
    }

    pub fn loaded_modules() -> Vec<String> {
        let mut modules: Vec<Handle> = vec![std::ptr::null_mut(); 1024];
        let mut needed = 0u32;
        // SAFETY: the buffer holds `modules.len()` handles; `needed` receives the byte count.
        let ok = unsafe {
            K32EnumProcessModules(GetCurrentProcess(), modules.as_mut_ptr(), (modules.len() * size_of::<Handle>()) as u32, &mut needed)
        };
        if ok == 0 {
            return Vec::new();
        }
        modules.truncate((needed as usize / size_of::<Handle>()).min(modules.len()));
        modules
            .iter()
            .filter_map(|&m| {
                let mut name = [0u16; 1024];
                // SAFETY: `name` holds 1024 characters.
                let n = unsafe { GetModuleFileNameW(m, name.as_mut_ptr(), name.len() as u32) } as usize;
                (n > 0).then(|| String::from_utf16_lossy(&name[..n.min(name.len())]))
            })
            .collect()
    }

    // ---- test hooks ----

    pub fn trigger_access_violation() {
        let bad = std::hint::black_box(0x10usize) as *mut u32;
        // SAFETY: deliberately not safe: this is the crash test hook (WTE_DEBUG_NATIVE_CRASH=av).
        unsafe { std::ptr::write_volatile(bad, 0xDEAD) };
    }

    pub fn trigger_heap_corruption() {
        // SAFETY: deliberately not safe: a double free on the process heap (WTE_DEBUG_NATIVE_CRASH=heap).
        unsafe {
            let heap = GetProcessHeap();
            let block = HeapAlloc(heap, 0, 64);
            HeapFree(heap, 0, block);
            HeapFree(heap, 0, std::hint::black_box(block));
            // A second round in case the first double free went unnoticed.
            let other = HeapAlloc(heap, 0, 64);
            std::ptr::write_bytes((other as *mut u8).wrapping_sub(16), 0xAB, 96);
            HeapFree(heap, 0, other);
            HeapFree(heap, 0, other);
        }
    }

    // ---- handlers ----

    /// Distinct addresses of first-chance fatal-kind exceptions already logged.
    static SEEN: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
    static SEEN_FULL_LOGGED: AtomicBool = AtomicBool::new(false);
    /// Thread id of the thread writing the crash report (0 = none).
    static CRASHING: AtomicU32 = AtomicU32::new(0);

    /// Records (never handles) first-chance exceptions of the fatal kinds; heap corruption and fail-fast are not
    /// continuable and never reach the unhandled filter, so they are reported as the crash here.
    unsafe extern "system" fn vectored_handler(pointers: *mut ExceptionPointers) -> i32 {
        // SAFETY: the system passes valid exception pointers.
        let Some(record) = (unsafe { pointers.as_ref().and_then(|p| p.record.as_ref()) }) else { return EXCEPTION_CONTINUE_SEARCH };
        match record.code {
            HEAP_CORRUPTION | STACK_BUFFER_OVERRUN => crash(pointers, false),
            // Almost no stack is left: the unhandled filter reports it from another thread.
            STACK_OVERFLOW => {}
            ACCESS_VIOLATION | ILLEGAL_INSTRUCTION => {
                let address = record.address as usize;
                if CRASHING.load(Ordering::SeqCst) != 0 {
                    return EXCEPTION_CONTINUE_SEARCH;
                }
                let mut new = false;
                for slot in &SEEN {
                    match slot.compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => {
                            new = true;
                            break;
                        }
                        Err(existing) if existing == address => break,
                        Err(_) => {}
                    }
                }
                let full = SEEN.iter().all(|s| s.load(Ordering::SeqCst) != 0);
                if new {
                    let mut line = StackBuf::<600>::new();
                    let _ = std::fmt::Write::write_fmt(&mut line, format_args!("[native] first-chance {:#010X} ({}) at ", record.code, code_name(record.code)));
                    push_location(&mut line, address);
                    let _ = std::fmt::Write::write_fmt(&mut line, format_args!(", thread {}", unsafe { GetCurrentThreadId() }));
                    push_access(&mut line, record);
                    line.push(b"\r\n");
                    note(line.as_bytes());
                    write_log(line.as_bytes());
                } else if full && !SEEN_FULL_LOGGED.swap(true, Ordering::SeqCst) {
                    write_log(b"[native] further first-chance exceptions at other addresses are not logged\r\n");
                }
            }
            _ => {}
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    unsafe extern "system" fn unhandled_filter(pointers: *mut ExceptionPointers) -> i32 {
        // SAFETY: the system passes valid exception pointers.
        let code = unsafe { pointers.as_ref().and_then(|p| p.record.as_ref()).map_or(0, |r| r.code) };
        if code == STACK_OVERFLOW {
            // Report from a fresh thread with a normal stack; this thread waits.
            let mut job = OverflowJob { pointers, thread_id: unsafe { GetCurrentThreadId() } };
            // SAFETY: `job` outlives the thread because this thread waits for it.
            let thread = unsafe { CreateThread(std::ptr::null_mut(), 1 << 20, overflow_thread, (&raw mut job).cast(), 0, std::ptr::null_mut()) };
            if !thread.is_null() {
                // Bounded: a new thread can block in DLL_THREAD_ATTACH when this thread holds the loader lock.
                unsafe {
                    WaitForSingleObject(thread, OVERFLOW_WAIT_MS);
                }
            }
            // Only reached when the thread could not be created or did not finish in time.
            unsafe { TerminateProcess(GetCurrentProcess(), code) };
        }
        crash(pointers, true)
    }

    struct OverflowJob {
        pointers: *mut ExceptionPointers,
        thread_id: u32,
    }

    unsafe extern "system" fn overflow_thread(param: *mut c_void) -> u32 {
        // SAFETY: `param` is the `OverflowJob` of the waiting faulting thread.
        let job = unsafe { &*(param as *const OverflowJob) };
        report(job.pointers, job.thread_id, true)
    }

    /// Writes the crash report for the current thread and ends the process.
    fn crash(pointers: *mut ExceptionPointers, from_filter: bool) -> ! {
        let code = report(pointers, unsafe { GetCurrentThreadId() }, from_filter);
        unsafe { TerminateProcess(GetCurrentProcess(), code) };
        loop {
            unsafe { Sleep(INFINITE) };
        }
    }

    /// The report: log + crash-<time>.txt, minidump, message box (not in self-test). Returns the exit code.
    fn report(pointers: *mut ExceptionPointers, thread_id: u32, from_filter: bool) -> u32 {
        // SAFETY: valid pointers from the system.
        let record = unsafe { pointers.as_ref().and_then(|p| p.record.as_ref()) };
        let code = record.map_or(0xC000_0001, |r| r.code);
        let address = record.map_or(0, |r| r.address as usize);
        let me = unsafe { GetCurrentThreadId() };
        if let Err(owner) = CRASHING.compare_exchange(0, me, Ordering::SeqCst, Ordering::SeqCst) {
            if owner == me {
                // A crash while writing the report: give up.
                unsafe { TerminateProcess(GetCurrentProcess(), code) };
            }
            // Another thread is reporting and will end the process.
            loop {
                unsafe { Sleep(INFINITE) };
            }
        }
        let setup = SETUP.get();
        let selftest = setup.is_some_and(|s| s.selftest);
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());

        let mut text = StackBuf::<8192>::new();
        let _ = std::fmt::Write::write_fmt(
            &mut text,
            format_args!(
                "\r\n===== NATIVE CRASH =====\r\nWrapped Texture Editor {} crashed: exception {:#010X} ({}){}\r\naddress: ",
                env!("CARGO_PKG_VERSION"),
                code,
                code_name(code),
                if from_filter { "" } else { " [reported from the vectored handler: not continuable]" },
            ),
        );
        push_location(&mut text, address);
        if let Some(r) = record {
            push_access(&mut text, r);
        }
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("\r\nthread: {thread_id}"));
        if code != HEAP_CORRUPTION {
            push_thread_name(&mut text, thread_id == me);
        }
        if let Some(start) = START.get() {
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("\r\nuptime: {:.1} s", start.elapsed().as_secs_f64()));
        }
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("\r\nunix time: {unix}\r\nlast actions and log lines (oldest first):\r\n"));
        push_ring(&mut text);

        // Report file: the first crash directory that works.
        let mut txt_path = [0u16; 600];
        let mut dmp_path = [0u16; 600];
        let mut file = INVALID_HANDLE_VALUE;
        if let Some(setup) = setup {
            for dir in &setup.dirs {
                let (Some(_), Some(_)) = (make_path(&mut txt_path, dir, unix, ".txt"), make_path(&mut dmp_path, dir, unix, ".dmp")) else { continue };
                let mut dir_z = [0u16; 600];
                if dir.len() < dir_z.len() {
                    dir_z[..dir.len()].copy_from_slice(dir);
                    unsafe { CreateDirectoryW(dir_z.as_ptr(), std::ptr::null_mut()) };
                }
                file = unsafe {
                    CreateFileW(txt_path.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ, std::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut())
                };
                if file != INVALID_HANDLE_VALUE {
                    break;
                }
            }
        }
        if file != INVALID_HANDLE_VALUE {
            text.push(b"crash report: ");
            text.push_wide(until_nul(&txt_path));
            text.push(b"\r\n");
        } else {
            text.push(b"crash report: could not create a file in the crash folder (this log entry is the full report)\r\n");
        }
        write_log(text.as_bytes());
        write_handle(file, text.as_bytes());

        // Minidump.
        let mut line = StackBuf::<1024>::new();
        if file != INVALID_HANDLE_VALUE {
            let dump = unsafe {
                CreateFileW(dmp_path.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut())
            };
            if dump == INVALID_HANDLE_VALUE {
                let _ = std::fmt::Write::write_fmt(&mut line, format_args!("minidump: cannot create the file (error {})\r\n", unsafe { GetLastError() }));
            } else {
                let info = MinidumpExceptionInformation { thread_id, exception_pointers: pointers, client_pointers: 0 };
                let ok = unsafe { MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), dump, DUMP_TYPE, &info, std::ptr::null(), std::ptr::null()) };
                let error = unsafe { GetLastError() };
                unsafe { CloseHandle(dump) };
                if ok != 0 {
                    line.push(b"minidump: ");
                    line.push_wide(until_nul(&dmp_path));
                    line.push(b"\r\n");
                } else {
                    let _ = std::fmt::Write::write_fmt(&mut line, format_args!("minidump: MiniDumpWriteDump failed (error {error:#010X})\r\n"));
                }
            }
        } else {
            line.push(b"minidump: skipped (no crash folder)\r\n");
        }
        line.push(b"===== END OF CRASH REPORT =====\r\n");
        write_log(line.as_bytes());
        write_handle(file, line.as_bytes());
        if file != INVALID_HANDLE_VALUE {
            unsafe {
                FlushFileBuffers(file);
                CloseHandle(file);
            }
        }
        if let Some(log) = setup.and_then(|s| s.log.as_ref()) {
            unsafe { FlushFileBuffers(log.as_raw_handle()) };
        }

        if !selftest {
            show_box(if file != INVALID_HANDLE_VALUE { Some(until_nul(&txt_path)) } else { None });
        }
        code
    }

    /// `<dir>\crash-<unix>-<pid><ext>` with a terminating NUL into `out`.
    fn make_path(out: &mut [u16; 600], dir: &[u16], unix: u64, ext: &str) -> Option<()> {
        let mut name = StackBuf::<64>::new();
        // The process id keeps two crashes in the same second apart.
        let pid = unsafe { GetCurrentProcessId() };
        let _ = std::fmt::Write::write_fmt(&mut name, format_args!("\\crash-{unix}-{pid}{ext}"));
        let total = dir.len() + name.len;
        if total + 1 > out.len() {
            return None;
        }
        out[..dir.len()].copy_from_slice(dir);
        for (i, b) in name.as_bytes().iter().enumerate() {
            out[dir.len() + i] = u16::from(*b);
        }
        out[total] = 0;
        Some(())
    }

    fn until_nul(s: &[u16]) -> &[u16] {
        &s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())]
    }

    /// `module.dll+0x1234 (C:\path\module.dll)` or the bare address.
    fn push_location<const N: usize>(out: &mut StackBuf<N>, address: usize) {
        let _ = std::fmt::Write::write_fmt(out, format_args!("{address:#x}"));
        let mut module: Handle = std::ptr::null_mut();
        // SAFETY: FROM_ADDRESS takes the address as the name; UNCHANGED_REFCOUNT needs no FreeLibrary.
        let ok = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address as *const u16,
                &mut module,
            )
        };
        if ok == 0 || module.is_null() {
            out.push(b" (not inside a loaded module)");
            return;
        }
        let mut name = [0u16; 520];
        // SAFETY: `name` holds 520 characters.
        let n = (unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize).min(name.len());
        let path = &name[..n];
        let file = path.iter().rposition(|c| *c == u16::from(b'\\') || *c == u16::from(b'/')).map_or(path, |i| &path[i + 1..]);
        out.push(b" = ");
        out.push_wide(file);
        let _ = std::fmt::Write::write_fmt(out, format_args!("+{:#x} (module base {:#x}, ", address.wrapping_sub(module as usize), module as usize));
        out.push_wide(path);
        out.push(b")");
    }

    /// For an access violation: read / write / execute of which address.
    fn push_access<const N: usize>(out: &mut StackBuf<N>, record: &ExceptionRecord) {
        if (record.code == ACCESS_VIOLATION || record.code == 0xC000_0006) && record.number_parameters >= 2 {
            let what = match record.information[0] {
                0 => "reading",
                1 => "writing",
                8 => "executing",
                _ => "accessing",
            };
            let _ = std::fmt::Write::write_fmt(out, format_args!(" ({what} address {:#x})", record.information[1]));
        }
    }

    /// `, name '<thread description>'` when the thread has one (GetThreadDescription, looked up at run time).
    fn push_thread_name<const N: usize>(out: &mut StackBuf<N>, current: bool) {
        type GetThreadDescription = unsafe extern "system" fn(Handle, *mut *mut u16) -> i32;
        if !current {
            return;
        }
        let name: [u16; 13] = [107, 101, 114, 110, 101, 108, 51, 50, 46, 100, 108, 108, 0]; // "kernel32.dll"
        // SAFETY: NUL-terminated strings; the function type matches GetThreadDescription.
        unsafe {
            let module = GetModuleHandleW(name.as_ptr());
            if module.is_null() {
                return;
            }
            let proc = GetProcAddress(module, c"GetThreadDescription".as_ptr().cast());
            if proc.is_null() {
                return;
            }
            let f: GetThreadDescription = std::mem::transmute(proc);
            let mut desc: *mut u16 = std::ptr::null_mut();
            if f(GetCurrentThread(), &mut desc) >= 0 && !desc.is_null() {
                let mut len = 0;
                while len < 256 && *desc.add(len) != 0 {
                    len += 1;
                }
                if len > 0 {
                    out.push(b" '");
                    out.push_wide(std::slice::from_raw_parts(desc, len));
                    out.push(b"'");
                }
                LocalFree(desc.cast());
            }
        }
    }

    /// The ring buffer, oldest line first.
    fn push_ring<const N: usize>(out: &mut StackBuf<N>) {
        let next = RING_NEXT.load(Ordering::Acquire);
        let first = next.saturating_sub(RING_SLOTS);
        for i in first..next {
            let slot = &RING[i % RING_SLOTS];
            let len = usize::from(slot.len.load(Ordering::Acquire)).min(RING_LINE);
            let mut line = [0u8; RING_LINE];
            for (j, b) in line.iter_mut().enumerate().take(len) {
                *b = slot.bytes[j].load(Ordering::Relaxed);
            }
            out.push(b"  ");
            out.push(&line[..len]);
            out.push(b"\r\n");
        }
    }

    fn write_log(bytes: &[u8]) {
        if let Some(log) = SETUP.get().and_then(|s| s.log.as_ref()) {
            write_handle(log.as_raw_handle(), bytes);
        }
    }

    fn write_handle(handle: Handle, bytes: &[u8]) {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return;
        }
        let mut written = 0u32;
        // SAFETY: `bytes` is valid for its length; the handle is open for writing.
        unsafe { WriteFile(handle, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut()) };
    }

    /// The message box, on its own thread (a modal loop on a crashed UI thread would re-enter winit's window
    /// procedure); this thread waits without pumping messages.
    fn show_box(report: Option<&[u16]>) {
        static mut TEXT: [u16; 900] = [0; 900];
        let mut msg = [0u16; 900];
        let mut n = 0;
        let mut push = |s: &[u16]| {
            for c in s {
                if n + 1 < msg.len() {
                    msg[n] = *c;
                    n += 1;
                }
            }
        };
        let ascii = |s: &str, f: &mut dyn FnMut(&[u16])| {
            for b in s.bytes() {
                f(&[u16::from(b)]);
            }
        };
        match report {
            Some(path) => {
                ascii("Wrapped Texture Editor crashed. A crash report was saved to ", &mut push);
                push(path);
                ascii(".\n\nPlease send this file (and the .dmp file next to it) together with the log file.", &mut push);
            }
            None => ascii("Wrapped Texture Editor crashed. The crash report could not be saved; see the log file.", &mut push),
        }
        msg[n] = 0;
        // SAFETY: only the single crashing thread (CRASHING) gets here, once.
        unsafe {
            let text = &raw mut TEXT;
            (*text).copy_from_slice(&msg);
            let thread = CreateThread(std::ptr::null_mut(), 256 << 10, box_thread, text.cast(), 0, std::ptr::null_mut());
            if !thread.is_null() {
                if WaitForSingleObject(thread, BOX_WAIT_MS) != 0 {
                    write_log(b"[native] crash message box not closed in time; ending the process\r\n");
                }
                CloseHandle(thread);
            }
        }
    }

    unsafe extern "system" fn box_thread(param: *mut c_void) -> u32 {
        let mut title = [0u16; 40];
        for (i, b) in "Wrapped Texture Editor - crash".bytes().enumerate() {
            title[i] = u16::from(b);
        }
        write_log(b"[native] showing the crash message box\r\n");
        // SAFETY: `param` is the NUL-terminated message text.
        unsafe { MessageBoxW(std::ptr::null_mut(), param as *const u16, title.as_ptr(), MB_ICONERROR | MB_TOPMOST) };
        write_log(b"[native] crash message box closed\r\n");
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_the_last_lines_and_truncates() {
        for i in 0..40 {
            note(format!("line {i}\n").as_bytes());
        }
        let long = "é".repeat(300);
        note(long.as_bytes());
        let next = RING_NEXT.load(Ordering::SeqCst);
        let slot = &RING[(next - 1) % RING_SLOTS];
        let len = usize::from(slot.len.load(Ordering::SeqCst));
        assert!(len <= RING_LINE && len % 2 == 0, "cut at a character boundary: {len}");
    }

    #[test]
    fn stack_buf_never_overflows() {
        let mut b = StackBuf::<8>::new();
        let _ = std::fmt::Write::write_str(&mut b, "0123456789");
        assert_eq!(b.as_bytes(), b"01234567");
    }
}
