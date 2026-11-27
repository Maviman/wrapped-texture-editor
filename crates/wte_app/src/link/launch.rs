//! Starting the outside editor (STICKER_PLAN 5.7, 13.1; stickers P9a).
//!
//! The command is built by a pure function, [`command_for`]: program = the editor's exe, one argument = the file
//! path (never through a shell, no quoting by hand, never `ShellExecute` or the file association), working folder =
//! the exe's folder. [`RealLauncher`] runs it with stdin / stdout / stderr null and `CREATE_NO_WINDOW` and keeps the
//! child; [`StubLauncher`] records the call and starts nothing. **A self-test run and every `#[cfg(test)]` build get
//! only the stub** ([`launcher_for`]; `RealLauncher` is not even compiled into a test build): no test, scenario or
//! agent ever starts PhotoCraft or Photoshop (rule 3 of 0.4).
//!
//! Discovery (5.7, 13.1, review A19): the registry only, read-only, through an injected [`RegistryReader`]: for each
//! editor its keys in order, each under `HKCU` then `HKLM`, each in the 64-bit then the 32-bit registry view; the exe
//! of the value must exist. PhotoCraft: two `shell\open\command` keys of the installed build; Photoshop (stickers
//! P9b): the App Paths key (a plain path) and `Applications\Photoshop.exe\shell\open\command` (a quoted path and
//! `"%1"`). No guessed folders. The portable PhotoCraft leaves no trace: the app then asks once with a file dialog;
//! "Other program..." is only ever the user's own choice.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::editor::ExternalEditor;

/// What starting an editor runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchCommand {
    pub program: OsString,
    /// Exactly one argument: the file.
    pub args: Vec<OsString>,
    /// The exe's folder (what `ShellExecute` and the App Paths key's `Path` value would set).
    pub cwd: Option<PathBuf>,
}

/// The command that opens `file` in `editor` (pure; spawns nothing). The same for every kind of editor: the exe with
/// the file as its one argument.
pub fn command_for(editor: &ExternalEditor, exe: &Path, file: &Path) -> LaunchCommand {
    let _ = editor;
    LaunchCommand {
        program: exe.as_os_str().to_owned(),
        args: vec![file.as_os_str().to_owned()],
        cwd: exe.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf),
    }
}

/// What is known about a launched editor process.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChildState {
    Running,
    Exited,
    /// Nothing can be said (no child kept, or `try_wait` failed).
    Unknown,
}

/// A launched editor process.
pub trait ChildProc: Send {
    fn state(&mut self) -> ChildState;
    /// Stops the process (the command-line conversion that took too long).
    fn kill(&mut self) {}
}

/// Starts editors and shows files (5.7).
pub trait Launcher: Send {
    /// Opens `file` in `editor` (`exe`). `Ok(Some(child))` when a process was started and kept.
    fn open(&mut self, editor: &ExternalEditor, exe: &Path, file: &Path) -> Result<Option<Box<dyn ChildProc>>, String>;
    /// "Show file in Explorer": a folder window with the file selected.
    fn show_file(&mut self, file: &Path) -> Result<(), String>;
    /// Starts `cli convert <input> <output>` (5.7: PhotoCraft's command-line tool, headless).
    fn convert(&mut self, cli: &Path, input: &Path, output: &Path) -> Result<Box<dyn ChildProc>, String>;
    /// True for the stub (tests and self-test runs).
    fn is_stub(&self) -> bool;
    /// The calls the stub saw (empty for the real launcher).
    fn calls(&self) -> Vec<StubCall> {
        Vec::new()
    }
}

/// One call the stub saw: (editor key, exe, file) for an open; ("explorer", "", file) for "Show file".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubCall {
    pub editor: String,
    pub exe: PathBuf,
    pub file: PathBuf,
}

/// Records every call and starts nothing. With `child_running` set it hands out a fake child whose state is that flag
/// (the two-copies guard of 5.7 in tests); without it the child is unknown, like the installed PhotoCraft build. A
/// conversion writes `convert_png` as its output (tests), or fails.
#[derive(Default)]
pub struct StubLauncher {
    pub log: Arc<Mutex<Vec<StubCall>>>,
    pub child_running: Option<Arc<AtomicBool>>,
    pub convert_png: Option<Vec<u8>>,
}

/// The command of `cli convert <input> <output>` (pure; spawns nothing).
pub fn convert_command(cli: &Path, input: &Path, output: &Path) -> LaunchCommand {
    LaunchCommand {
        program: cli.as_os_str().to_owned(),
        args: vec!["convert".into(), input.as_os_str().to_owned(), output.as_os_str().to_owned()],
        cwd: cli.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf),
    }
}

struct FakeChild(Arc<AtomicBool>);

impl ChildProc for FakeChild {
    fn state(&mut self) -> ChildState {
        if self.0.load(Ordering::SeqCst) { ChildState::Running } else { ChildState::Exited }
    }
}

impl Launcher for StubLauncher {
    fn open(&mut self, editor: &ExternalEditor, exe: &Path, file: &Path) -> Result<Option<Box<dyn ChildProc>>, String> {
        let command = command_for(editor, exe, file);
        log::info!("stub launcher: would start {:?} with {:?} in {:?}", command.program, command.args, command.cwd);
        if let Ok(mut log) = self.log.lock() {
            log.push(StubCall { editor: editor.key().to_owned(), exe: exe.to_path_buf(), file: file.to_path_buf() });
        }
        Ok(self.child_running.as_ref().map(|flag| Box::new(FakeChild(Arc::clone(flag))) as Box<dyn ChildProc>))
    }

    fn show_file(&mut self, file: &Path) -> Result<(), String> {
        log::info!("stub launcher: would show {} in Explorer", file.display());
        if let Ok(mut log) = self.log.lock() {
            log.push(StubCall { editor: "explorer".to_owned(), exe: PathBuf::new(), file: file.to_path_buf() });
        }
        Ok(())
    }

    fn convert(&mut self, cli: &Path, input: &Path, output: &Path) -> Result<Box<dyn ChildProc>, String> {
        log::info!("stub launcher: would run {:?}", convert_command(cli, input, output));
        if let Ok(mut log) = self.log.lock() {
            log.push(StubCall { editor: "convert".to_owned(), exe: cli.to_path_buf(), file: input.to_path_buf() });
        }
        let png = self.convert_png.as_ref().ok_or("the stub launcher converts nothing")?;
        std::fs::write(output, png).map_err(|e| e.to_string())?;
        Ok(Box::new(FakeChild(Arc::new(AtomicBool::new(false)))))
    }

    fn is_stub(&self) -> bool {
        true
    }

    fn calls(&self) -> Vec<StubCall> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

/// The launcher of an interactive run: the stub in a self-test run and in every test build.
pub fn launcher_for(selftest: bool) -> Box<dyn Launcher> {
    if !selftest && let Some(real) = real_launcher() {
        return real;
    }
    Box::new(StubLauncher::default())
}

/// The real launcher (a non-test build).
#[cfg(not(test))]
fn real_launcher() -> Option<Box<dyn Launcher>> {
    Some(Box::new(real::RealLauncher))
}

/// A test build has no real launcher at all.
#[cfg(test)]
fn real_launcher() -> Option<Box<dyn Launcher>> {
    None
}

/// The real launcher: only in a non-test build, only constructed by `launcher_for(false)`.
#[cfg(not(test))]
mod real {
    use super::*;

    /// Starts the editor for real (an interactive run, the user's own click).
    pub struct RealLauncher;

    struct RealChild(std::process::Child);

    impl ChildProc for RealChild {
        fn state(&mut self) -> ChildState {
            match self.0.try_wait() {
                Ok(None) => ChildState::Running,
                Ok(Some(_)) => ChildState::Exited,
                Err(_) => ChildState::Unknown,
            }
        }

        fn kill(&mut self) {
            let _ = self.0.kill();
        }
    }

    /// A headless process: stdio null, `CREATE_NO_WINDOW`, working folder from the command.
    fn spawn(c: &LaunchCommand) -> std::io::Result<std::process::Child> {
        let mut cmd = std::process::Command::new(&c.program);
        cmd.args(&c.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if let Some(dir) = &c.cwd {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn()
    }

    impl Launcher for RealLauncher {
        fn open(&mut self, editor: &ExternalEditor, exe: &Path, file: &Path) -> Result<Option<Box<dyn ChildProc>>, String> {
            log::info!("starting {} for {}", exe.display(), file.display());
            let child = spawn(&command_for(editor, exe, file)).map_err(|e| format!("{} could not be started: {e}", editor.name()))?;
            Ok(Some(Box::new(RealChild(child))))
        }

        fn convert(&mut self, cli: &Path, input: &Path, output: &Path) -> Result<Box<dyn ChildProc>, String> {
            log::info!("converting {} with {}", input.display(), cli.display());
            let child = spawn(&convert_command(cli, input, output)).map_err(|e| format!("{} could not be started: {e}", cli.display()))?;
            Ok(Box::new(RealChild(child)))
        }

        fn show_file(&mut self, file: &Path) -> Result<(), String> {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let mut cmd = std::process::Command::new("explorer.exe");
                cmd.raw_arg(format!("/select,\"{}\"", file.display()));
                cmd.spawn().map(|_| ()).map_err(|e| format!("Explorer could not be started: {e}"))
            }
            #[cfg(not(windows))]
            {
                Err(format!("showing {} is only possible on Windows", file.display()))
            }
        }

        fn is_stub(&self) -> bool {
            false
        }
    }
}

/// A registry root.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// A registry view (13.1): the 64-bit one (`RRF_SUBKEY_WOW6464KEY`) and the 32-bit one (`RRF_SUBKEY_WOW6432KEY`) of a
/// 64-bit Windows. Discovery tries the 64-bit view first.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RegView {
    Bits64,
    Bits32,
}

/// Reads registry values (read-only; injected so tests never touch the real registry).
pub trait RegistryReader: Send {
    /// The default value of `subkey` in `view` as a string, `None` when the key or value is missing.
    fn default_value(&self, hive: Hive, view: RegView, subkey: &str) -> Option<String>;
}

/// A registry with nothing in it (self-test runs and tests).
pub struct NoRegistry;

impl RegistryReader for NoRegistry {
    fn default_value(&self, _hive: Hive, _view: RegView, _subkey: &str) -> Option<String> {
        None
    }
}

/// The real registry: `advapi32` `RegGetValueW` through `raw-dylib` (read only).
pub struct WindowsRegistry;

impl RegistryReader for WindowsRegistry {
    #[cfg(windows)]
    fn default_value(&self, hive: Hive, view: RegView, subkey: &str) -> Option<String> {
        use std::ffi::c_void;
        const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
        const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
        const RRF_RT_REG_SZ: u32 = 0x0000_0002;
        const RRF_SUBKEY_WOW6464KEY: u32 = 0x0001_0000;
        const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;
        #[link(name = "advapi32", kind = "raw-dylib")]
        unsafe extern "system" {
            fn RegGetValueW(
                hkey: isize,
                subkey: *const u16,
                value: *const u16,
                flags: u32,
                kind: *mut u32,
                data: *mut c_void,
                len: *mut u32,
            ) -> i32;
        }
        let root = match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        };
        let flags = RRF_RT_REG_SZ
            | match view {
                RegView::Bits64 => RRF_SUBKEY_WOW6464KEY,
                RegView::Bits32 => RRF_SUBKEY_WOW6432KEY,
            };
        let key: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut len: u32 = 0;
        // SAFETY: valid NUL-terminated key, a null value name (the default value), null data to ask for the size.
        let status = unsafe { RegGetValueW(root, key.as_ptr(), std::ptr::null(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut len) };
        if status != 0 || !(2..=64 * 1024).contains(&len) {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2)];
        // SAFETY: `buf` holds `len` bytes; the call writes at most `len` bytes and updates it.
        let status = unsafe {
            RegGetValueW(root, key.as_ptr(), std::ptr::null(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
        };
        if status != 0 {
            return None;
        }
        let chars = (len as usize / 2).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..chars]);
        Some(text.trim_end_matches('\0').to_owned())
    }

    #[cfg(not(windows))]
    fn default_value(&self, _hive: Hive, _view: RegView, _subkey: &str) -> Option<String> {
        None
    }
}

/// The registry reader of a run: the real one only in an interactive run of a non-test build.
pub fn registry_for(selftest: bool) -> Box<dyn RegistryReader> {
    if selftest || cfg!(test) { Box::new(NoRegistry) } else { Box::new(WindowsRegistry) }
}

/// PhotoCraft's two keys (5.7; `PCHEAD:` installer `photocraft.wxs:83` and `:109`), each tried under `HKCU`, then
/// `HKLM`.
const PHOTOCRAFT_KEYS: [&str; 2] =
    [r"Software\Classes\Photocraft.PhotoshopDocument\shell\open\command", r"Software\Classes\Applications\photocraft.exe\shell\open\command"];

/// The exe of a `shell\open\command` line or an App Paths value: the quoted part when the line starts with a quote,
/// else everything up to and including the first ".exe", else the whole trimmed line.
pub fn exe_of_command(line: &str) -> Option<PathBuf> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        let end = rest.find('"')?;
        let exe = rest[..end].trim();
        return (!exe.is_empty()).then(|| PathBuf::from(exe));
    }
    let lower = line.to_ascii_lowercase();
    let exe = match lower.find(".exe") {
        Some(i) => &line[..i + 4],
        None => line,
    };
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// Photoshop's two keys (13.1): the App Paths key (default value = the exe, a plain path; `Path` names its folder) and
/// the `Applications` key (a quoted path followed by `"%1"`), each tried under `HKCU`, then `HKLM`.
const PHOTOSHOP_KEYS: [&str; 2] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Photoshop.exe",
    r"Software\Classes\Applications\Photoshop.exe\shell\open\command",
];

/// The registry keys an editor is discovered from (none for another program: only the user picks it).
pub fn registry_keys(editor: &ExternalEditor) -> &'static [&'static str] {
    match editor {
        ExternalEditor::PhotoCraft => &PHOTOCRAFT_KEYS,
        ExternalEditor::Photoshop => &PHOTOSHOP_KEYS,
        ExternalEditor::Other(_) => &[],
    }
}

/// Discovery of `editor` (5.7, 13.1): the first registry entry whose exe exists, key by key, `HKCU` before `HKLM`, the
/// 64-bit view before the 32-bit one. Another program is never discovered.
pub fn discover(editor: &ExternalEditor, reader: &dyn RegistryReader, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    for key in registry_keys(editor) {
        for hive in [Hive::CurrentUser, Hive::LocalMachine] {
            for view in [RegView::Bits64, RegView::Bits32] {
                if let Some(exe) = reader.default_value(hive, view, key).as_deref().and_then(exe_of_command)
                    && exists(&exe)
                {
                    return Some(exe);
                }
            }
        }
    }
    None
}
