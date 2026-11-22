// Owner: B5 brush flows (round 4; before: app-shell A1). Preferences file of ARCHITECTURE.md 8.9: JSON at
// %APPDATA%\WrappedTextureEditor\prefs.json. Failures are logged and ignored; the self-test never calls `load` / `save`.
// Round 4: the brush and eraser settings, the remembered brush shape and how its openings are closed (FEATURE_BRUSH
// 4.4, 4.5; ROUND4_PLAN section 7 item 5).

use std::path::{Path, PathBuf};

use wte_core::CloseMode;

use crate::state::Prefs;

/// Maximum number of entries in `Prefs::recent`.
pub const MAX_RECENT: usize = 10;
/// Largest remembered window side in points. A larger value (a hand-edited file, a window stretched over several
/// monitors) is clamped; the first frame then fits the window onto its monitor (`app::fit_to_monitor`).
pub const MAX_WINDOW_SIDE: f32 = 8192.0;
/// Most shape files whose closing choice is remembered (`Prefs::brush_shape_close`); the remembered shape's own entry
/// is always kept.
pub const MAX_SHAPE_CLOSE: usize = 32;

/// `%APPDATA%\WrappedTextureEditor` (None when APPDATA is not set).
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").filter(|v| !v.is_empty()).map(|v| PathBuf::from(v).join("WrappedTextureEditor"))
}

fn prefs_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("prefs.json"))
}

/// Reads `%APPDATA%\WrappedTextureEditor\prefs.json`; defaults when it is missing or unreadable.
pub fn load() -> Prefs {
    match prefs_path() {
        Some(path) => load_from(&path),
        None => {
            log::warn!("APPDATA is not set; using default preferences");
            Prefs::default()
        }
    }
}

/// Writes the preferences file; failures are logged and ignored.
pub fn save(prefs: &Prefs) {
    match prefs_path() {
        Some(path) => save_to(prefs, &path),
        None => log::warn!("APPDATA is not set; preferences are not saved"),
    }
}

/// Reads a preferences file; defaults (logged) when it is missing or invalid.
pub fn load_from(path: &Path) -> Prefs {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Prefs::default(),
        Err(e) => {
            log::warn!("cannot read {}: {e}", path.display());
            return Prefs::default();
        }
    };
    match from_json(&text) {
        Ok(prefs) => prefs,
        Err(e) => {
            log::warn!("invalid preferences file {}: {e}", path.display());
            Prefs::default()
        }
    }
}

/// Writes a preferences file (directory created); failures are logged.
pub fn save_to(prefs: &Prefs, path: &Path) {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, to_json(prefs))
    })();
    if let Err(e) = result {
        log::warn!("cannot write {}: {e}", path.display());
    }
}

/// Pretty JSON of the preferences.
pub fn to_json(prefs: &Prefs) -> String {
    serde_json::to_string_pretty(prefs).unwrap_or_else(|_| "{}".to_owned())
}

/// Parses preferences JSON; missing fields take their defaults, the recent list is trimmed to `MAX_RECENT`.
pub fn from_json(text: &str) -> Result<Prefs, serde_json::Error> {
    let mut prefs: Prefs = serde_json::from_str(text)?;
    prefs.recent.truncate(MAX_RECENT);
    let [w, h] = prefs.window_size;
    if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0) {
        prefs.window_size = Prefs::default().window_size;
    }
    prefs.window_size = prefs.window_size.map(|v| v.min(MAX_WINDOW_SIDE));
    // Values that read but are out of range are forced into range (FEATURE_BRUSH 4.5; the lenient reading of
    // `state.rs` sanitizes too, this keeps the rule in one visible place).
    prefs.brush = prefs.brush.sanitized();
    prefs.eraser = prefs.eraser.sanitized();
    prefs.brush_shape_close.retain(|_, close| *close != CloseMode::Refuse);
    trim_close(&mut prefs);
    Ok(prefs)
}

/// How the openings of the shape file `path` are closed without asking: the user's stored answer, else
/// `CloseMode::Refuse` (ask when it has openings).
pub fn stored_close(prefs: &Prefs, path: &Path) -> CloseMode {
    prefs.brush_shape_close.get(path).copied().unwrap_or(CloseMode::Refuse)
}

/// Stores the user's answer for the shape file `path` (a later load or reload closes it the same way without asking);
/// `Refuse` forgets it. At most `MAX_SHAPE_CLOSE` files are remembered.
pub fn remember_close(prefs: &mut Prefs, path: &Path, close: CloseMode) {
    if close == CloseMode::Refuse {
        forget_close(prefs, path);
        return;
    }
    prefs.brush_shape_close.insert(path.to_path_buf(), close);
    let keep = path.to_path_buf();
    while prefs.brush_shape_close.len() > MAX_SHAPE_CLOSE {
        let Some(drop) = prefs.brush_shape_close.keys().find(|p| **p != keep && Some(*p) != prefs.brush_shape_path.as_ref()).cloned()
        else {
            break;
        };
        prefs.brush_shape_close.remove(&drop);
    }
}

/// Forgets the stored answer for `path` (the next load asks again).
pub fn forget_close(prefs: &mut Prefs, path: &Path) {
    prefs.brush_shape_close.remove(path);
}

/// Keeps at most `MAX_SHAPE_CLOSE` stored answers (a hand-edited file), the remembered shape's first.
fn trim_close(prefs: &mut Prefs) {
    while prefs.brush_shape_close.len() > MAX_SHAPE_CLOSE {
        let Some(drop) = prefs.brush_shape_close.keys().rev().find(|p| Some(*p) != prefs.brush_shape_path.as_ref()).cloned()
        else {
            break;
        };
        prefs.brush_shape_close.remove(&drop);
    }
}

/// Moves `path` to the front of the recent-files list (no duplicates, at most `MAX_RECENT`).
pub fn add_recent(prefs: &mut Prefs, path: &Path) {
    prefs.recent.retain(|p| p != path);
    prefs.recent.insert(0, path.to_path_buf());
    prefs.recent.truncate(MAX_RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_through_a_file() {
        let dir = wte_core::testutil::temp_dir("prefs_round_trip");
        let path = dir.join("sub").join("prefs.json");
        let prefs = Prefs {
            recent: vec![PathBuf::from(r"E:\p\a.wtep"), PathBuf::from(r"E:\p\b.wtep")],
            last_dir: Some(PathBuf::from(r"E:\models")),
            last_export_dir: Some(PathBuf::from(r"E:\export")),
            window_size: [1500.0, 900.0],
            window_maximized: true,
            flags_wireframe: true,
            keep_backup: true,
            swatches: vec![[1, 2, 3, 255]; 24],
            ..Prefs::default()
        };
        save_to(&prefs, &path);
        assert_eq!(load_from(&path), prefs);
        assert_eq!(from_json(&to_json(&prefs)).unwrap(), prefs);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_invalid_files_give_defaults() {
        let dir = wte_core::testutil::temp_dir("prefs_invalid");
        assert_eq!(load_from(&dir.join("nope.json")), Prefs::default());
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        assert_eq!(load_from(&bad), Prefs::default());
        std::fs::remove_dir_all(&dir).ok();

        let p = from_json(r#"{ "window_size": [-1, 5], "recent": ["1","2","3","4","5","6","7","8","9","10","11","12"] }"#).unwrap();
        assert_eq!(p.window_size, Prefs::default().window_size);
        assert_eq!(p.recent.len(), MAX_RECENT);
        let p = from_json(r#"{ "window_size": [9000, 900], "window_maximized": true }"#).unwrap();
        assert_eq!(p.window_size, [MAX_WINDOW_SIDE, 900.0]);
        assert!(p.window_maximized);
    }

    #[test]
    fn recent_list_is_most_recent_first_without_duplicates() {
        let mut prefs = Prefs::default();
        for i in 0..12 {
            add_recent(&mut prefs, Path::new(&format!("f{i}.wtep")));
        }
        assert_eq!(prefs.recent.len(), MAX_RECENT);
        assert_eq!(prefs.recent[0], PathBuf::from("f11.wtep"));
        add_recent(&mut prefs, Path::new("f5.wtep"));
        assert_eq!(prefs.recent[0], PathBuf::from("f5.wtep"));
        assert_eq!(prefs.recent.iter().filter(|p| p.as_path() == Path::new("f5.wtep")).count(), 1);
        assert_eq!(prefs.recent.len(), MAX_RECENT);
    }

    /// A10 (FEATURE_BRUSH 8.2): type, shape, Use, angles, offset, align, the path and the closing choice survive
    /// `to_json` / `from_json` and a file; old, foreign and hand-edited texts load with the rest kept.
    #[test]
    fn a10_brush_preferences_round_trip_and_read_leniently() {
        use crate::state::{BrushSettings, BrushType, ShapeChoice};
        let brush = BrushSettings {
            size_px: 77.0,
            hardness: 0.25,
            kind: BrushType::Sphere,
            shape: ShapeChoice::File,
            through: true,
            turn_deg: 30.0,
            tilt_deg: -45.0,
            roll_deg: 170.0,
            offset: -0.5,
            align: wte_core::TipAlign::World,
            ..BrushSettings::default()
        };
        let eraser = BrushSettings { shape: ShapeChoice::Cylinder, kind: BrushType::Sphere, ..BrushSettings::default() };
        let shape = PathBuf::from(r"E:\shapes\star.obj");
        let mut prefs = Prefs {
            recent: vec![PathBuf::from(r"E:\p\a.wtep")],
            brush: brush.clone(),
            eraser: eraser.clone(),
            brush_shape_path: Some(shape.clone()),
            ..Prefs::default()
        };
        remember_close(&mut prefs, &shape, CloseMode::RoundedCaps);
        let back = from_json(&to_json(&prefs)).unwrap();
        assert_eq!(back, prefs);
        assert_eq!(back.brush, brush);
        assert_eq!(back.eraser, eraser);
        assert_eq!(stored_close(&back, &shape), CloseMode::RoundedCaps);
        assert_eq!(stored_close(&back, Path::new(r"E:\other.obj")), CloseMode::Refuse);

        // A file of round 3 (no brush keys): default brushes, the recent files kept.
        let old = from_json(r#"{ "recent": ["E:\\p\\a.wtep"], "keep_backup": true }"#).unwrap();
        assert_eq!((old.brush.clone(), old.eraser.clone()), (BrushSettings::default(), BrushSettings::default()));
        assert_eq!(old.recent, vec![PathBuf::from(r"E:\p\a.wtep")]);
        assert!(old.keep_backup && old.brush_shape_path.is_none() && old.brush_shape_close.is_empty());

        // A word of a later version: that brush is the default, the rest of the file is kept.
        let later = from_json(r#"{ "recent": ["x.wtep"], "brush": { "kind": "spiral" }, "eraser": { "size_px": 12 } }"#).unwrap();
        assert_eq!(later.brush, BrushSettings::default());
        assert_eq!(later.eraser.size_px, 12.0);
        assert_eq!(later.recent, vec![PathBuf::from("x.wtep")]);

        // Out of range: forced into range.
        let wild = from_json(r#"{ "brush": { "size_px": 10000000, "hardness": -3, "turn_deg": 900, "offset": 7 } }"#).unwrap();
        assert_eq!((wild.brush.size_px, wild.brush.hardness, wild.brush.turn_deg, wild.brush.offset), (2048.0, 0.0, 180.0, 1.0));

        // An unknown key loads; an unknown closing mode drops only its own entry; "refuse" is never stored.
        let extra = from_json(
            r#"{ "future_key": [1, 2], "brush_shape_path": "E:\\s\\a.obj",
                 "brush_shape_close": { "E:\\s\\a.obj": "flat_caps", "E:\\s\\b.obj": "glue", "E:\\s\\c.obj": "refuse" } }"#,
        )
        .unwrap();
        assert_eq!(extra.brush_shape_path, Some(PathBuf::from(r"E:\s\a.obj")));
        assert_eq!(extra.brush_shape_close.len(), 1);
        assert_eq!(stored_close(&extra, Path::new(r"E:\s\a.obj")), CloseMode::FlatCaps);

        // Through a file under target\wte-test-tmp.
        let dir = wte_core::testutil::temp_dir("prefs_a10");
        let path = dir.join("prefs.json");
        save_to(&prefs, &path);
        assert_eq!(load_from(&path), prefs);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The stored closing choices: `Refuse` forgets, at most `MAX_SHAPE_CLOSE` files, the remembered shape's entry and
    /// the newest are kept (also when a hand-edited file holds more).
    #[test]
    fn closing_choices_are_stored_per_file_and_bounded() {
        let mut prefs = Prefs::default();
        let mine = PathBuf::from(r"E:\s\000_mine.obj");
        prefs.brush_shape_path = Some(mine.clone());
        remember_close(&mut prefs, &mine, CloseMode::FillHollows);
        for i in 0..50 {
            remember_close(&mut prefs, Path::new(&format!(r"E:\s\f{i:02}.obj")), CloseMode::FlatCaps);
        }
        assert_eq!(prefs.brush_shape_close.len(), MAX_SHAPE_CLOSE);
        assert_eq!(stored_close(&prefs, &mine), CloseMode::FillHollows);
        assert_eq!(stored_close(&prefs, Path::new(r"E:\s\f49.obj")), CloseMode::FlatCaps);
        remember_close(&mut prefs, &mine, CloseMode::Refuse);
        assert_eq!(stored_close(&prefs, &mine), CloseMode::Refuse);
        assert!(!prefs.brush_shape_close.contains_key(&mine));

        let mut map = String::new();
        for i in 0..40 {
            map.push_str(&format!(r#""E:\\s\\g{i:02}.obj": "rounded_caps","#));
        }
        let text = format!(r#"{{ "brush_shape_path": "E:\\s\\g39.obj", "brush_shape_close": {{ {} }} }}"#, map.trim_end_matches(','));
        let read = from_json(&text).unwrap();
        assert_eq!(read.brush_shape_close.len(), MAX_SHAPE_CLOSE);
        assert_eq!(stored_close(&read, Path::new(r"E:\s\g39.obj")), CloseMode::RoundedCaps);
    }
}
