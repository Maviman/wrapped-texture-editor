//! USB through `adb reverse` (COMPANION_PLAN 6.1, decision C11): find adb.exe, parse `adb devices -l`, run
//! `adb -s SERIAL reverse tcp:PORT tcp:PORT` once per device appearance, and report a line for the dialog. Google's
//! platform-tools are not bundled; the user downloads them (or has Android Studio) and the app finds adb.exe.
//!
//! adb is always spawned with `CREATE_NO_WINDOW`, stdin null and piped output, so a console window never appears. The
//! app never kills a process: an adb call that does not answer within `CALL_TIMEOUT` is left to finish by itself, and
//! the adb server daemon that adb starts keeps running. A self-test run starts the watcher only for the one serial named
//! by `WTE_COMPANION_ADB_SERIAL` (decision C13); the user's real tablet is never addressed then.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::server::Hub;

/// How long one adb call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the watcher asks adb for the devices: without and with a session.
pub const POLL_IDLE: Duration = Duration::from_secs(2);
pub const POLL_SESSION: Duration = Duration::from_secs(5);
/// `CREATE_NO_WINDOW` (process creation flag).
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The download page named in the dialog and the Help text.
pub const PLATFORM_TOOLS_URL: &str = "https://developer.android.com/tools/releases/platform-tools";

/// The places adb.exe is looked for, in order (6.1): the preference, `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `PATH`,
/// `%LOCALAPPDATA%\Android\Sdk\platform-tools`, `platform-tools` next to the app's exe.
pub fn candidates(pref: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = pref {
        out.push(p.to_path_buf());
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = env(var).filter(|v| !v.is_empty()) {
            out.push(PathBuf::from(root).join("platform-tools").join("adb.exe"));
        }
    }
    if let Some(path) = env("PATH") {
        out.extend(std::env::split_paths(&path).filter(|d| !d.as_os_str().is_empty()).map(|d| d.join("adb.exe")));
    }
    if let Some(local) = env("LOCALAPPDATA").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(local).join("Android").join("Sdk").join("platform-tools").join("adb.exe"));
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join("platform-tools").join("adb.exe"));
    }
    out
}

/// The first candidate that exists.
pub fn find(pref: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    candidates(pref, env, exe_dir).into_iter().find(|p| exists(p))
}

/// `find` with the real environment.
pub fn find_here(pref: Option<&Path>) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    find(pref, &|v| std::env::var_os(v), exe_dir.as_deref(), &|p| p.is_file())
}

/// One line of `adb devices -l`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    /// `device`, `unauthorized`, `offline`, `no permissions`, ...
    pub state: String,
    /// The `model:` field, if any (shown in the dialog).
    pub model: Option<String>,
}

/// Parses `adb devices -l`: skips the header, daemon start lines and empty lines.
pub fn parse_devices(text: &str) -> Vec<Device> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') || line.starts_with("adb ") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            continue;
        }
        // The state is every word before the first `key:value` field ("no permissions" has two).
        let state_words: Vec<&str> = rest.iter().take_while(|w| !w.contains(':')).copied().collect();
        if state_words.is_empty() {
            continue;
        }
        let model = rest.iter().find_map(|w| w.strip_prefix("model:")).map(|m| m.replace('_', " "));
        out.push(Device { serial: serial.to_owned(), state: state_words.join(" "), model });
    }
    out
}

/// The arguments of `adb -s SERIAL reverse tcp:PORT tcp:PORT`.
pub fn reverse_args(serial: &str, port: u16) -> Vec<String> {
    vec!["-s".into(), serial.into(), "reverse".into(), format!("tcp:{port}"), format!("tcp:{port}")]
}

/// The arguments of `adb devices -l`.
pub fn devices_args() -> Vec<String> {
    vec!["devices".into(), "-l".into()]
}

/// Runs adb without a window; `Ok((status ok, stdout + stderr))`. An adb that does not answer within `timeout`, or
/// that is still running when `stop` says so (error `Interrupted`), is left running (never killed). The watcher passes
/// the server's stop flag, so stopping the server (a link turned off, the port changed, the app closing) never waits
/// for a slow adb on the UI thread.
pub fn run_until(adb: &Path, args: &[String], timeout: Duration, stop: &dyn Fn() -> bool) -> io::Result<(bool, String)> {
    let mut cmd = Command::new(adb);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut text);
        }
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut text);
        }
        text
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let text = reader.join().unwrap_or_default();
            return Ok((status.success(), text));
        }
        if start.elapsed() >= timeout {
            // Left to finish by itself (the reader thread ends when it does).
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!("adb {} did not answer", args.join(" "))));
        }
        if stop() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "the companion server stopped"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the dialog's USB line says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdbLine {
    Searching,
    NoAdb,
    /// adb found, no tablet attached.
    Found(PathBuf),
    /// A tablet is attached but has not allowed USB debugging yet.
    Unauthorized,
    Offline,
    /// The reverse rule is set for these devices (model or serial).
    Ready(Vec<String>),
    Failed(String),
}

impl AdbLine {
    /// The text of the dialog line (COMPANION_PLAN 3).
    pub fn text(&self) -> String {
        match self {
            AdbLine::Searching => "Looking for adb...".into(),
            AdbLine::NoAdb => "No adb found".into(),
            AdbLine::Found(p) => format!("adb found: {}. Connect the tablet with the cable.", p.display()),
            AdbLine::Unauthorized => "Tablet connected: unlock it and tap Allow USB debugging".into(),
            AdbLine::Offline => "The tablet is offline: reconnect the cable".into(),
            AdbLine::Ready(names) => format!("Ready ({}): open Tablet Companion on the tablet and choose USB", names.join(", ")),
            AdbLine::Failed(e) => format!("adb failed: {e}"),
        }
    }
}

/// The line for a device list (pure; `reversed` = the serials whose reverse rule is set).
pub fn line_for(devices: &[Device], reversed: &[String], adb: &Path) -> AdbLine {
    let ready: Vec<String> = devices
        .iter()
        .filter(|d| d.state == "device" && reversed.contains(&d.serial))
        .map(|d| d.model.clone().unwrap_or_else(|| d.serial.clone()))
        .collect();
    if !ready.is_empty() {
        AdbLine::Ready(ready)
    } else if devices.iter().any(|d| d.state == "unauthorized") {
        AdbLine::Unauthorized
    } else if devices.iter().any(|d| d.state == "offline") {
        AdbLine::Offline
    } else {
        AdbLine::Found(adb.to_path_buf())
    }
}

/// The watcher's line, shared with the UI.
pub type SharedLine = Arc<Mutex<AdbLine>>;

/// Starts the watcher thread (stops with the server, `Hub::is_stopped`). `only_serial` = the one serial a self-test
/// run may address.
pub fn watch(hub: Arc<Hub>, adb_pref: Option<PathBuf>, port: u16, only_serial: Option<String>) -> (JoinHandle<()>, SharedLine) {
    let line: SharedLine = Arc::new(Mutex::new(AdbLine::Searching));
    let out = Arc::clone(&line);
    let set = move |l: AdbLine| *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = l;
    let thread = super::server::spawn_named("wte-companion-adb", move || {
        // Serials whose reverse rule is set in their current appearance.
        let mut reversed: Vec<String> = Vec::new();
        let mut next = Instant::now();
        while !hub.is_stopped() {
            if Instant::now() < next {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            next = Instant::now() + if hub.session().is_some() { POLL_SESSION } else { POLL_IDLE };
            let Some(adb) = find_here(adb_pref.as_deref()) else {
                set(AdbLine::NoAdb);
                reversed.clear();
                continue;
            };
            let stop = || hub.is_stopped();
            let devices = match run_until(&adb, &devices_args(), CALL_TIMEOUT, &stop) {
                Ok((_, text)) => parse_devices(&text),
                Err(e) => {
                    set(AdbLine::Failed(e.to_string()));
                    continue;
                }
            };
            let devices: Vec<Device> = devices.into_iter().filter(|d| only_serial.as_ref().is_none_or(|s| *s == d.serial)).collect();
            // A device that left (or is no longer authorized) gets the rule again when it comes back.
            reversed.retain(|s| devices.iter().any(|d| d.serial == *s && d.state == "device"));
            let new: Vec<String> =
                devices.iter().filter(|d| d.state == "device" && !reversed.contains(&d.serial)).map(|d| d.serial.clone()).collect();
            for serial in new {
                if hub.is_stopped() {
                    break;
                }
                match run_until(&adb, &reverse_args(&serial, port), CALL_TIMEOUT, &stop) {
                    Ok((true, _)) => {
                        log::info!("companion: adb reverse tcp:{port} set for {serial}");
                        reversed.push(serial);
                    }
                    Ok((false, text)) => log::warn!("companion: adb reverse for {serial} failed: {}", text.trim()),
                    Err(e) => log::warn!("companion: adb reverse for {serial}: {e}"),
                }
            }
            set(line_for(&devices, &reversed, &adb));
        }
    });
    (thread, line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call that is still running gives up within a poll step once the stop flag is set (Server::stop joins the
    /// watcher on the UI thread). `ping -n 4 127.0.0.1` stands in for a slow adb (about 3 s, no window).
    #[cfg(windows)]
    #[test]
    fn a_running_call_gives_up_when_the_server_stops() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&stop);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            s2.store(true, Ordering::SeqCst);
        });
        let t0 = Instant::now();
        let args: Vec<String> = ["-n", "4", "127.0.0.1"].iter().map(|s| (*s).to_owned()).collect();
        let r = run_until(Path::new("ping"), &args, Duration::from_secs(10), &|| stop.load(Ordering::SeqCst));
        let took = t0.elapsed();
        assert!(matches!(&r, Err(e) if e.kind() == io::ErrorKind::Interrupted), "{r:?}");
        assert!(took < Duration::from_millis(1500), "gave up after {took:?}");
        // Without a stop the same call finishes normally.
        let r = run_until(Path::new("ping"), &["-n".to_owned(), "1".to_owned(), "127.0.0.1".to_owned()], Duration::from_secs(10), &|| false);
        assert!(matches!(r, Ok((true, _))), "{r:?}");
    }

    #[test]
    fn devices_list_parser() {
        let text = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\n\
                    R52T1234ABC            device usb:1-1 product:gts8uwifixx model:SM_X900 device:gts8u transport_id:3\n\
                    emulator-5554          device product:sdk_gtablet_x86_64 model:sdk_gtablet_x86_64 device:emu64xa transport_id:1\n\
                    R52T9999               unauthorized usb:1-2 transport_id:4\n\
                    0123456789             offline transport_id:5\n\
                    ABCDEF                 no permissions (user in plugdev group) usb:1-3\n\n";
        let d = parse_devices(text);
        assert_eq!(d.len(), 5, "{d:?}");
        assert_eq!(d[0], Device { serial: "R52T1234ABC".into(), state: "device".into(), model: Some("SM X900".into()) });
        assert_eq!(d[1].serial, "emulator-5554");
        assert_eq!(d[1].state, "device");
        assert_eq!(d[2].state, "unauthorized");
        assert_eq!(d[3].state, "offline");
        assert_eq!(d[4].state, "no permissions (user in plugdev group)");
        assert!(parse_devices("List of devices attached\n\n").is_empty());
        assert!(parse_devices("").is_empty());
        // Without -l: serial and state only.
        assert_eq!(parse_devices("List of devices attached\nemulator-5554\tdevice\n")[0].state, "device");
    }

    #[test]
    fn the_search_order() {
        let env = |v: &str| -> Option<OsString> {
            match v {
                "ANDROID_HOME" => Some("C:\\sdk".into()),
                "ANDROID_SDK_ROOT" => Some("".into()),
                "PATH" => Some("C:\\a;C:\\b".into()),
                "LOCALAPPDATA" => Some("C:\\Users\\u\\AppData\\Local".into()),
                _ => None,
            }
        };
        let pref = PathBuf::from("D:\\tools\\adb.exe");
        let c = candidates(Some(&pref), &env, Some(Path::new("C:\\app")));
        let want: Vec<PathBuf> = [
            "D:\\tools\\adb.exe",
            "C:\\sdk\\platform-tools\\adb.exe",
            "C:\\a\\adb.exe",
            "C:\\b\\adb.exe",
            "C:\\Users\\u\\AppData\\Local\\Android\\Sdk\\platform-tools\\adb.exe",
            "C:\\app\\platform-tools\\adb.exe",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(c, want);
        // The first that exists wins; none = None.
        let found = find(Some(&pref), &env, Some(Path::new("C:\\app")), &|p| p.starts_with("C:\\b") || p.starts_with("C:\\app"));
        assert_eq!(found, Some(PathBuf::from("C:\\b\\adb.exe")));
        assert_eq!(find(None, &|_| None, None, &|_| true), None);
    }

    #[test]
    fn command_lines_and_lines() {
        assert_eq!(reverse_args("emulator-5554", 47811), ["-s", "emulator-5554", "reverse", "tcp:47811", "tcp:47811"]);
        assert_eq!(devices_args(), ["devices", "-l"]);
        let adb = Path::new("C:\\sdk\\adb.exe");
        let dev = |s: &str, st: &str| Device { serial: s.into(), state: st.into(), model: None };
        assert_eq!(line_for(&[], &[], adb), AdbLine::Found(adb.into()));
        assert_eq!(line_for(&[dev("a", "unauthorized")], &[], adb), AdbLine::Unauthorized);
        assert_eq!(line_for(&[dev("a", "offline")], &[], adb), AdbLine::Offline);
        assert_eq!(line_for(&[dev("a", "device"), dev("b", "unauthorized")], &["a".into()], adb), AdbLine::Ready(vec!["a".into()]));
        assert!(AdbLine::Ready(vec!["SM X900".into()]).text().contains("Tablet Companion"));
        assert_eq!(AdbLine::NoAdb.text(), "No adb found");
    }
}
