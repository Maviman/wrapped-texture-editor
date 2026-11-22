// Owner: app-shell (A1). Hand-written command line parser of ARCHITECTURE.md 8.11.

use std::ffi::OsString;
use std::path::PathBuf;

/// Self-test window size when `--size` is not given.
pub const DEFAULT_SIZE: [u32; 2] = [1280, 800];
/// Scenario when `--scenario` is not given.
pub const DEFAULT_SCENARIO: &str = "default";

/// Command line: `wrapped_texture_editor.exe [PATH] [--screenshot OUT.png] [--scenario NAME] [--size WxH] [--log FILE]`.
#[derive(Clone, Debug, PartialEq)]
pub struct CliArgs {
    /// `.wtep` project or model file opened at startup.
    pub path: Option<PathBuf>,
    /// Self-test mode: the screenshot to write.
    pub screenshot: Option<PathBuf>,
    /// Self-test scenario; defaults to `default`.
    pub scenario: String,
    /// Self-test window size in physical pixels; defaults to 1280x800.
    pub size: [u32; 2],
    /// Log file.
    pub log: Option<PathBuf>,
}

impl CliArgs {
    /// The screenshot path without its `.png` extension (`None` outside self-test mode).
    pub fn out_stem(&self) -> Option<PathBuf> {
        self.screenshot.as_ref().map(|p| p.with_extension(""))
    }
}

/// Parses the arguments (without the program name). They are OS strings, so a file name that is not valid Unicode
/// (an unpaired UTF-16 surrogate, possible on NTFS) still opens instead of panicking in `std::env::args`.
pub fn parse(args: impl Iterator<Item = OsString>) -> Result<CliArgs, String> {
    let mut out = CliArgs { path: None, screenshot: None, scenario: DEFAULT_SCENARIO.to_owned(), size: DEFAULT_SIZE, log: None };
    let mut scenario_given = false;
    let mut size_given = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value_os = |flag: &str| -> Result<OsString, String> {
            match args.next() {
                Some(v) if !v.is_empty() => Ok(v),
                _ => Err(format!("{flag} needs a value")),
            }
        };
        let text = |v: OsString, flag: &str| v.into_string().map_err(|v| format!("{flag} {} is not valid Unicode", v.display()));
        match arg.to_str().unwrap_or("") {
            "--screenshot" => {
                if out.screenshot.is_some() {
                    return Err("--screenshot is given twice".to_owned());
                }
                out.screenshot = Some(PathBuf::from(value_os("--screenshot")?));
            }
            "--scenario" => {
                if scenario_given {
                    return Err("--scenario is given twice".to_owned());
                }
                out.scenario = text(value_os("--scenario")?, "--scenario")?;
                scenario_given = true;
            }
            "--size" => {
                if size_given {
                    return Err("--size is given twice".to_owned());
                }
                out.size = parse_size(&text(value_os("--size")?, "--size")?)?;
                size_given = true;
            }
            "--log" => {
                if out.log.is_some() {
                    return Err("--log is given twice".to_owned());
                }
                out.log = Some(PathBuf::from(value_os("--log")?));
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ if arg.is_empty() => return Err("empty argument".to_owned()),
            _ => {
                if out.path.is_some() {
                    return Err(format!("more than one file given ({})", arg.display()));
                }
                out.path = Some(PathBuf::from(arg));
            }
        }
    }
    if out.screenshot.is_none() && (scenario_given || size_given) {
        return Err("--scenario and --size need --screenshot".to_owned());
    }
    Ok(out)
}

/// `WxH` (also `wxh`), both sides 64..=16384.
fn parse_size(text: &str) -> Result<[u32; 2], String> {
    let bad = || format!("invalid --size {text:?} (expected WxH, e.g. 1280x800)");
    let (w, h) = text.split_once(['x', 'X']).ok_or_else(bad)?;
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if !(64..=16384).contains(&w) || !(64..=16384).contains(&h) {
        return Err(format!("--size {w}x{h} is out of range (64..=16384 per side)"));
    }
    Ok([w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<CliArgs, String> {
        parse(args.iter().map(OsString::from))
    }

    /// A file name with an unpaired surrogate is not valid Unicode: it is still the file to open (a panic in
    /// `std::env::args` before), and a non-Unicode value of a text option is an error, not a panic.
    #[cfg(windows)]
    #[test]
    fn non_unicode_file_names_are_accepted() {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = r"E:\m\lone_".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(".obj".encode_utf16());
        let name = OsString::from_wide(&wide);
        assert!(name.to_str().is_none());
        let a = parse([name.clone(), "--screenshot".into(), name.clone()].into_iter()).unwrap();
        assert_eq!(a.path, Some(PathBuf::from(&name)));
        assert_eq!(a.screenshot, Some(PathBuf::from(&name)));
        assert!(parse(["--screenshot".into(), "s.png".into(), "--scenario".into(), name].into_iter()).is_err());
    }

    #[test]
    fn no_arguments_give_the_defaults() {
        let a = p(&[]).unwrap();
        assert_eq!(a, CliArgs { path: None, screenshot: None, scenario: "default".into(), size: [1280, 800], log: None });
        assert_eq!(a.out_stem(), None);
    }

    #[test]
    fn a_single_path_is_the_file_to_open() {
        let a = p(&[r"E:\models\a b.wtep"]).unwrap();
        assert_eq!(a.path, Some(PathBuf::from(r"E:\models\a b.wtep")));
        assert!(a.screenshot.is_none());
    }

    #[test]
    fn self_test_options_in_any_order() {
        let a = p(&["--size", "800x600", "--scenario", "smoke", "--screenshot", r"E:\out\shot.png", "--log", r"E:\out\x.log", "model.obj"])
            .unwrap();
        assert_eq!(a.screenshot, Some(PathBuf::from(r"E:\out\shot.png")));
        assert_eq!(a.scenario, "smoke");
        assert_eq!(a.size, [800, 600]);
        assert_eq!(a.log, Some(PathBuf::from(r"E:\out\x.log")));
        assert_eq!(a.path, Some(PathBuf::from("model.obj")));
        assert_eq!(a.out_stem(), Some(PathBuf::from(r"E:\out\shot")));

        let b = p(&["--screenshot", "s.png"]).unwrap();
        assert_eq!(b.scenario, "default");
        assert_eq!(b.size, [1280, 800]);
        assert_eq!(p(&["--screenshot", "s.png", "--size", "1920X1080"]).unwrap().size, [1920, 1080]);
    }

    #[test]
    fn errors() {
        assert!(p(&["--screenshot"]).is_err());
        assert!(p(&["--screenshot", ""]).is_err());
        assert!(p(&["--screenshot", "a.png", "--size", "100"]).is_err());
        assert!(p(&["--screenshot", "a.png", "--size", "axb"]).is_err());
        assert!(p(&["--screenshot", "a.png", "--size", "10x10"]).is_err());
        assert!(p(&["--screenshot", "a.png", "--screenshot", "b.png"]).is_err());
        assert!(p(&["--frobnicate"]).is_err());
        assert!(p(&["a.obj", "b.obj"]).is_err());
        assert!(p(&["--scenario", "smoke"]).is_err(), "--scenario without --screenshot");
    }
}
