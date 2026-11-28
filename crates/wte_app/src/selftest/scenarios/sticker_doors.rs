// Stickers S2 (STICKER_PLAN 7 "S2", 8.2; ARCHITECTURE.md 8.11, 8.18): the two more doors in the real window, on a
// project saved into the run folder; every file is the run's own (`<run>_links\`, `<run>_assets\doc-assets\`) and is
// written the way a program's save lands (temp file + rename, or in place). Nothing is started.
//
// - `sticker_live_copy`: (1) a tray sticker from a picture file; before the project is saved its "Keep a PNG copy up
//   to date" box is disabled; (2) Save As, the box ticked (through `ScreenClick`): `<run>_links\badge_k<id>.png` is the
//   sticker's picture, no history entry; (3) the sticker edited in the Sticker view (a nudge): the copy is not written
//   at once, then once, as the new picture; (4) undo: written again as the first picture; (5) the paint layer "Paint
//   1" (a red stroke) ticked in its row menu: `<run>_links\Paint 1_l<id>.png` is the layer at the texture's size, the
//   Layers tab shows its line; (6) save, reopen: both copies are back and found equal (not written again); (7) the
//   tray box unticked: the copy stays on disk and is no longer updated.
// - `sticker_watch_folder`: (1) a folder with two pictures watched (File > Watch Folder for Stickers... with its
//   path): two kept tray stickers "badge" and "star", one entry "Stickers from Folder"; (2) the badge placed on the
//   sphere; (3) the "editor" saves a new badge.png (temp + rename): the same sticker updates in place, one entry
//   "Update Sticker from Folder", the stamp's placement unchanged, the model shows the new colour; (4) a wider badge.png
//   written in place: the artboard follows, the stamp stays a user of the same sticker; (5) undo, 1.5 s: nothing comes
//   back by itself; "Read again" applies it once (the unchanged star makes no entry); (6) undo everything gives the
//   start back, redo everything the end; (7) save, reopen: the folder is watched again and nothing is applied again;
//   (8) Stop watching keeps the stickers.
// Each shot says what it must show.

use super::sticker_tab::{doc_hash, hit_click, reveal};
use super::*;
use crate::doors::live::{LiveStatus, LiveTarget};
use crate::doors::{DoorAction, FOLDER_BATCH_LABEL, FOLDER_UPDATE_LABEL};
use crate::sticker::{StickerAction, tray};

/// Brings control `hit` of the upper page into view (two wheel steps at most, each let settle) and clicks it.
macro_rules! tap {
    ($name:literal, $hit:expr) => {
        [
            Step::Dyn(concat!("show ", $name), |s, _| reveal(s, $hit)),
            Step::WaitMs(300),
            Step::WaitFrames(3),
            Step::Dyn(concat!("show ", $name), |s, _| reveal(s, $hit)),
            // The page's scroll animation ends before the rect is read.
            Step::WaitMs(300),
            Step::WaitFrames(3),
            Step::Dyn(concat!("click ", $name), |s, _| hit_click(s, $hit)),
            Step::WaitFrames(3),
        ]
    };
}

fn project(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, ".wtep")
}

fn sticker_key(s: &AppState, sc: &Scratch, key: &str) -> Result<wte_core::StickerId, String> {
    let id = wte_core::StickerId(remembered(sc, key)?);
    doc_of(s)?.sticker(id).ok_or_else(|| format!("the sticker \"{key}\" is gone"))?;
    Ok(id)
}

fn decode(path: &Path) -> Result<PixelBuf, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    wte_core::decode_image(&bytes, None, 8192).map(|d| d.pixels).map_err(|e| e.to_string())
}

fn png(p: &PixelBuf) -> Result<Vec<u8>, String> {
    wte_core::encode_png(p.width, p.height, &p.data, PngLevel::Fast).map_err(|e| e.to_string())
}

/// A `w x h` badge: a disc of `rgba` with a dark ring on transparency.
fn badge(w: u32, h: u32, rgba: [u8; 4]) -> PixelBuf {
    let mut p = PixelBuf::new(w, h);
    let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 / 2.0 - 1.0);
    for y in 0..h {
        for x in 0..w {
            let d = Vec2::new(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy).length();
            if d <= r {
                p.set(x, y, if d > r - 3.0 { [30, 30, 40, 255] } else { rgba });
            }
        }
    }
    p
}

fn writes(s: &AppState) -> u64 {
    s.doors.live.counters.writes
}

fn copy_path(s: &AppState, target: LiveTarget) -> Result<PathBuf, String> {
    crate::doors::live::path_of(s, target).ok_or_else(|| "no copy".to_owned())
}

/// The copy of the sticker remembered as "sticker" is that sticker's picture.
fn copy_is_the_sticker(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let id = sticker_key(s, sc, "sticker")?;
    let path = copy_path(s, LiveTarget::Sticker(id))?;
    let want = s.doc_mut().ok_or("no document")?.sticker_picture(id, None, 1.0).map_err(|e| e.to_string())?;
    if decode(&path)? == want { Ok(()) } else { Err(format!("{} is not the sticker's picture", path.display())) }
}

pub(super) fn sticker_live_copy() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("write the badge picture file; no links folder yet; the Sticker tab", |s, sc| {
            let dir = fresh_assets_dir(sc)?;
            write(&dir.join("badge.png"), &png(&badge(96, 96, [240, 170, 20, 255]))?)?;
            let links = wte_core::link::links_folder(&project(sc));
            if links.exists() {
                std::fs::remove_dir_all(&links).map_err(|e| e.to_string())?;
            }
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::Dyn("New Sticker from the picture file", |_, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::NewStickerFromPicture(Some(assets_dir(sc).join("badge.png")))))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Setup("remember the tray sticker and the history", |s, sc| {
            let [id] = tray::tray_stickers(s)[..] else { return Err("not one tray sticker".into()) };
            sc.values.insert("sticker", id.0);
            remember_undo(s, sc)
        }),
    ];
    // (1) Not saved yet: the box is drawn but disabled; a click changes nothing.
    steps.extend(tap!("the copy box (not saved)", PanelHit::LiveCopyTray));
    steps.extend([
        Step::Assert("not saved: no copy, no entry", |s, sc| {
            if !s.doors.live.entries.is_empty() {
                return Err("a copy was turned on".into());
            }
            undo_since(s, sc, 0)
        }),
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
    ]);
    // (2) Saved: ticked.
    steps.extend(tap!("Keep a PNG copy up to date", PanelHit::LiveCopyTray));
    steps.extend([
        Step::Until("the first copy is written", |s, _| Ok(writes(s) == 1)),
        Step::Until("it is up to date", |s, sc| {
            let id = sticker_key(s, sc, "sticker")?;
            Ok(s.doors.live.entries.get(&LiveTarget::Sticker(id)).is_some_and(|e| e.status == LiveStatus::UpToDate))
        }),
        Step::Setup("the copy: badge_k<id>.png in the links folder, the sticker's picture; no entry; modified", |s, sc| {
            let id = sticker_key(s, sc, "sticker")?;
            let path = copy_path(s, LiveTarget::Sticker(id))?;
            let want = wte_core::link::links_folder(&project(sc)).join(format!("badge_k{}.png", id.0));
            if path != want {
                return Err(format!("the copy is {}", path.display()));
            }
            copy_is_the_sticker(s, sc)?;
            if !doc_of(s)?.is_modified() {
                return Err("turning it on did not mark the project modified".into());
            }
            undo_since(s, sc, 0)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "live_copy_on",
            expect: "3D view of the plain sphere; side panel: the Sticker tab's tray with one thumbnail (the orange badge with \
                     a dark ring, selected), Selected: badge, the box Keep a PNG copy up to date ticked and the line PNG \
                     copy: badge_k<id>.png, up to date with a Show file button, then Watch a folder...",
        },
        // (3) An edit in the Sticker view: written again after the delay.
        Step::Dyn("open the tray sticker in the Sticker view", |s, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::EnterTray(sticker_key(s, sc, "sticker")?)))])
        }),
        Step::WaitIdle,
        Step::Setup("remember", remember_undo),
        Step::Do(AppAction::Sticker(StickerAction::Nudge(12, 0))),
        Step::Assert("one entry; the copy is not written at once", |s, sc| {
            if writes(s) != 1 {
                return Err(format!("{} writes right after the change", writes(s)));
            }
            undo_since(s, sc, 1)
        }),
        Step::Until("the copy is written again", |s, _| Ok(writes(s) == 2)),
        Step::Setup("the copy is the nudged picture", copy_is_the_sticker),
        Step::WaitFrames(3),
        Step::Shot {
            name: "live_copy_edited",
            expect: "Sticker view: the canvas shows the badge moved a little to the right (its right edge cut by the \
                     artboard); the 3D half shows the plain sphere; side panel: the Sticker tab's tray (badge selected) \
                     with Keep a PNG copy up to date ticked and PNG copy: badge_k<id>.png, up to date; the Layers tab \
                     with the one row badge; status line: Not placed yet",
        },
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Until("the copy is written again after the undo", |s, _| Ok(writes(s) == 3)),
        Step::Setup("the copy is the first picture again", |s, sc| {
            copy_is_the_sticker(s, sc)?;
            let dir = assets_dir(sc);
            let id = sticker_key(s, sc, "sticker")?;
            let copy = decode(&copy_path(s, LiveTarget::Sticker(id))?)?;
            if copy != decode(&dir.join("badge.png"))? {
                return Err("the copy after the undo is not the original badge".into());
            }
            Ok(())
        }),
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::WaitIdle,
        // (5) The paint layer.
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("a red stroke across the sphere on Paint 1", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.6, -0.3], [-0.2, -0.36], [0.25, -0.3], [0.6, -0.12]])
        }),
        Step::WaitIdle,
        Step::Setup("remember Paint 1; the Layers tab", |s, sc| {
            let session = session_of(s)?;
            let layer = session.doc.set(session.active_set).and_then(|x| x.active_layer()).ok_or("no active layer")?;
            if layer.name != "Paint 1" || !layer.is_raster() {
                return Err(format!("the active layer is {}", layer.name));
            }
            sc.values.insert("paint", layer.id.0);
            s.side_tab = crate::state::SideTab::Layers;
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
        Step::Dyn("right-click the row of Paint 1", |s, sc| {
            let rect = panel_rect(s, PanelHit::LayerRow(LayerId(remembered(sc, "paint")?)))?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Secondary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitFrames(3),
        Step::Dyn("click Keep a PNG Copy up to Date", |s, _| {
            let rect = panel_rect(s, PanelHit::LayerMenuLiveCopy)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitFrames(3),
        Step::Until("the layer's copy is written", |s, _| Ok(writes(s) == 4)),
        Step::Setup("the layer's copy: Paint 1_l<id>.png, the layer at the texture's size; no entry", |s, sc| {
            let id = LayerId(remembered(sc, "paint")?);
            let path = copy_path(s, LiveTarget::Layer(id))?;
            if path.file_name().is_none_or(|n| n.to_string_lossy() != format!("Paint 1_l{}.png", id.0)) {
                return Err(format!("the copy is {}", path.display()));
            }
            let doc = doc_of(s)?;
            let [w, h] = doc.layer_texture_size(id).ok_or("no texture")?;
            let want = doc.layer(id).ok_or("no layer")?.to_dense(w, h);
            if decode(&path)? != want {
                return Err("the copy is not the layer's pixels".into());
            }
            undo_since(s, sc, 0)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "live_copy_layer",
            expect: "3D view: the sphere with a red stroke; side panel: the Layers tab with, at the top of the list, the \
                     box Keep a PNG copy up to date ticked and the line PNG copy: Paint 1_l<id>.png, up to date with Show \
                     file, then the rows (Paint 1 active)",
        },
        // (6) Save, reopen: both copies are back and found equal.
        Step::Do(AppAction::Save),
        Step::WaitIdle,
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Until("both copies are checked", |s, _| Ok(s.doors.live.counters.same == 2)),
        Step::Assert("both copies are back, not written again", |s, sc| {
            let id = wte_core::StickerId(remembered(sc, "sticker")?);
            let layer = LayerId(remembered(sc, "paint")?);
            let live = &s.doors.live;
            if live.entries.len() != 2 || !live.entries.contains_key(&LiveTarget::Sticker(id)) || !live.entries.contains_key(&LiveTarget::Layer(layer)) {
                return Err(format!("copies {:?}", live.entries.keys().collect::<Vec<_>>()));
            }
            if writes(s) != 4 || doc_of(s)?.is_modified() {
                return Err(format!("{} writes, modified {}", writes(s), doc_of(s)?.is_modified()));
            }
            Ok(())
        }),
        Step::Setup("the Sticker tab, the tray sticker selected", |s, sc| {
            s.upper_tab = UpperTab::Sticker;
            s.sticker_ui.tray_selected = Some(wte_core::StickerId(remembered(sc, "sticker")?));
            remember_undo(s, sc)
        }),
        Step::WaitFrames(3),
    ]);
    // (7) Unticked: the file stays.
    steps.extend(tap!("Keep a PNG copy up to date (off)", PanelHit::LiveCopyTray));
    steps.extend([Step::Assert("the sticker's copy is off; its file stays; no entry; the layer's copy is still on", |s, sc| {
        let id = wte_core::StickerId(remembered(sc, "sticker")?);
        if s.doors.live.entries.contains_key(&LiveTarget::Sticker(id)) || s.doors.live.entries.len() != 1 {
            return Err("the copy is still on".into());
        }
        let file = wte_core::link::links_folder(&project(sc)).join(format!("badge_k{}.png", id.0));
        if !file.is_file() {
            return Err(format!("{} was removed", file.display()));
        }
        undo_since(s, sc, 0)
    })]);
    steps
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_watch_folder
// ---------------------------------------------------------------------------------------------------------------------

fn folder(sc: &Scratch) -> PathBuf {
    assets_dir(sc).join("doc-assets")
}

fn counters(s: &AppState) -> &crate::doors::FolderCounters {
    &s.doors.folder_counters
}

fn file_sticker(s: &AppState, name: &str) -> Result<wte_core::StickerId, String> {
    let w = s.doors.watch.as_ref().ok_or("no folder is watched")?;
    let f = w.files.get(name).ok_or_else(|| format!("{name} has no sticker"))?;
    Ok(f.sticker)
}

/// What the "editor" saves: through a temp file and a rename.
fn save_via_rename(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp-save");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub(super) fn sticker_watch_folder() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("a folder with badge.png and star.png; remember the start; the Sticker tab", |s, sc| {
            fresh_assets_dir(sc)?;
            let dir = folder(sc);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            write(&dir.join("badge.png"), &png(&badge(96, 96, [240, 170, 20, 255]))?)?;
            write(&dir.join("star.png"), &png(&badge(64, 64, [60, 200, 90, 255]))?)?;
            write(&dir.join("readme.txt"), b"not a picture")?;
            sc.values.insert("start_undo", doc_of(s)?.history.undo_len() as u64);
            sc.values.insert("start_doc", doc_hash(doc_of(s)?));
            remember_undo(s, sc)?;
            s.doors.folder_times = crate::doors::watch::FolderTimes { poll: Duration::from_millis(200), ..Default::default() };
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the tray offers Watch a folder...", |s, _| panel_rect(s, PanelHit::TrayWatchFolder).map(|_| ())),
        // (1) Watched (the menu's folder dialog answered with the run's folder).
        Step::Dyn("Watch Folder for Stickers...", |_, sc| Ok(vec![Step::Do(AppAction::Doors(DoorAction::WatchFolder(Some(folder(sc)))))])),
        Step::Until("both pictures arrive", |s, _| Ok(counters(s).added == 2)),
        Step::WaitIdle,
        Step::Assert("two kept tray stickers badge and star; one entry Stickers from Folder", |s, sc| {
            let doc = doc_of(s)?;
            let mut names: Vec<String> = tray::tray_stickers(s).iter().filter_map(|id| doc.sticker(*id)).map(|d| d.name.clone()).collect();
            names.sort();
            if names != ["badge", "star"] {
                return Err(format!("tray {names:?}"));
            }
            if doc.undo_label() != Some(FOLDER_BATCH_LABEL) {
                return Err(format!("the last entry is {:?}", doc.undo_label()));
            }
            undo_since(s, sc, 1)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "watch_two",
            expect: "3D view of the plain sphere; side panel: the Sticker tab's tray with two thumbnails (an orange badge and \
                     a green one, each with a dark ring), then Watching: doc-assets, 2 pictures from it are in the tray, and \
                     the buttons Read again, Show folder, Stop watching; a toast about the watched folder",
        },
        // (2) The badge placed on the sphere.
        Step::Setup("the badge as the Stamp tool's source", |s, sc| {
            let id = file_sticker(s, "badge.png")?;
            sc.values.insert("badge", id.0);
            crate::sticker::tray::use_as_source(s, id);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the badge", |s, _| model_click(s, ViewId::Main3D, [0.0, -0.05])),
        Step::WaitIdle,
        Step::Setup("remember the stamp, its placement and the model", |s, sc| {
            let id = sticker_key(s, sc, "badge")?;
            let [stamp] = doc_of(s)?.sticker_users(id)[..] else { return Err("the badge is not placed once".into()) };
            sc.values.insert("stamp", stamp.0);
            sc.values.insert("placement", placement_of(s, stamp)?);
            remember_composite(s, sc, "placed")?;
            remember_undo(s, sc)
        }),
        // (3) The editor saves a new badge.png.
        Step::Setup("the editor saves a blue badge (temp file + rename)", |_, sc| {
            save_via_rename(&folder(sc).join("badge.png"), &png(&badge(96, 96, [40, 90, 240, 255]))?)
        }),
        Step::Until("the update arrives", |s, _| Ok(counters(s).updated == 1)),
        Step::WaitIdle,
        Step::Assert("the same sticker, one entry Update Sticker from Folder, the placement kept, the model changed", |s, sc| {
            let id = sticker_key(s, sc, "badge")?;
            if file_sticker(s, "badge.png")? != id {
                return Err("another sticker took the file".into());
            }
            let doc = doc_of(s)?;
            if doc.undo_label() != Some(FOLDER_UPDATE_LABEL) {
                return Err(format!("the last entry is {:?}", doc.undo_label()));
            }
            let stamp = StampId(remembered(sc, "stamp")?);
            if placement_of(s, stamp)? != remembered(sc, "placement")? {
                return Err("the stamp moved".into());
            }
            if composite_hash(doc) == remembered(sc, "placed")? {
                return Err("the model did not change".into());
            }
            undo_since(s, sc, 1)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "watch_updated",
            expect: "3D view: the sphere with the badge, now blue with a dark ring, selected in the Transform gizmo where the \
                     orange one was placed; side panel: the Sticker tab's Selected stamp badge (Sticker, 1 layer) with the \
                     blue thumbnail; the Layers tab with the row badge (3D stamp); a toast Updated the sticker of badge.png \
                     from the watched folder",
        },
        // (4) A wider badge written in place.
        Step::Setup("the editor writes a wider badge in place", |_, sc| {
            write(&folder(sc).join("badge.png"), &png(&badge(160, 96, [200, 40, 160, 255]))?)
        }),
        Step::Until("the second update arrives", |s, _| Ok(counters(s).updated == 2)),
        Step::WaitIdle,
        Step::Assert("the artboard is 160 x 96; the stamp still shows the same sticker", |s, sc| {
            let id = sticker_key(s, sc, "badge")?;
            let doc = doc_of(s)?;
            let size = doc.sticker(id).ok_or("no sticker")?.size();
            if size != [160, 96] || doc.sticker_users(id) != vec![StampId(remembered(sc, "stamp")?)] {
                return Err(format!("size {size:?}, users {:?}", doc.sticker_users(id)));
            }
            undo_since(s, sc, 2)
        }),
        // (5) Undo: nothing comes back by itself; Read again applies it.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::WaitMs(1500),
        Step::Assert("after the undo and 1.5 s: the blue badge, no new update", |s, sc| {
            let id = sticker_key(s, sc, "badge")?;
            if doc_of(s)?.sticker(id).ok_or("no sticker")?.size() != [96, 96] || counters(s).updated != 2 {
                return Err(format!("size {:?}, {} updates", doc_of(s)?.sticker(id).map(|d| d.size()), counters(s).updated));
            }
            undo_since(s, sc, 1)
        }),
    ];
    steps.extend(tap!("Read again", PanelHit::TrayWatchRescan));
    steps.extend([
        Step::Until("Read again applies the badge and finds the star unchanged", |s, _| Ok(counters(s).updated == 3 && counters(s).same == 1)),
        Step::WaitIdle,
        Step::Assert("the wide badge again: one entry", |s, sc| {
            let id = sticker_key(s, sc, "badge")?;
            if doc_of(s)?.sticker(id).ok_or("no sticker")?.size() != [160, 96] {
                return Err("Read again did not apply the file".into());
            }
            undo_since(s, sc, 2)
        }),
        // (6) Undo everything, redo everything.
        Step::Setup("remember the end", |s, sc| {
            sc.values.insert("end_doc", doc_hash(doc_of(s)?));
            sc.values.insert("end_undo", doc_of(s)?.history.undo_len() as u64);
            Ok(())
        }),
        Step::Dyn("undo everything", |s, sc| {
            let n = doc_of(s)?.history.undo_len() as u64 - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Undo)).chain([Step::WaitIdle]).collect())
        }),
        Step::WaitMs(1200),
        Step::Assert("after undoing everything: the start, and the folder applied nothing again", |s, sc| {
            if doc_hash(doc_of(s)?) != remembered(sc, "start_doc")? || !tray::tray_stickers(s).is_empty() {
                return Err("the document differs from the start".into());
            }
            if counters(s).added != 2 || counters(s).updated != 3 {
                return Err(format!("{:?}", counters(s)));
            }
            Ok(())
        }),
        Step::Dyn("redo everything", |_, sc| {
            let n = remembered(sc, "end_undo")? - remembered(sc, "start_undo")?;
            Ok((0..n).map(|_| Step::Do(AppAction::Redo)).chain([Step::WaitIdle]).collect())
        }),
        Step::Assert("after redoing everything: the end", |s, sc| {
            if doc_hash(doc_of(s)?) != remembered(sc, "end_doc")? { Err("the document differs from the end".into()) } else { Ok(()) }
        }),
        // (7) Save, reopen.
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Setup("remember the counters", |s, sc| {
            sc.values.insert("added", counters(s).added);
            sc.values.insert("updated", counters(s).updated);
            s.doors.folder_times = crate::doors::watch::FolderTimes { poll: Duration::from_millis(200), ..Default::default() };
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitMs(1500),
        Step::Dyn("show the watched folder's lines", |s, _| reveal(s, PanelHit::TrayWatchRescan)),
        Step::WaitMs(300),
        Step::WaitFrames(3),
        Step::Assert("reopened: the folder is watched, its two files known, nothing applied again, not modified", |s, sc| {
            let w = s.doors.watch.as_ref().ok_or("the folder is not watched")?;
            if w.folder != folder(sc) || w.files.len() != 2 {
                return Err(format!("{} files of {}", w.files.len(), w.folder.display()));
            }
            if counters(s).added != remembered(sc, "added")? || counters(s).updated != remembered(sc, "updated")? {
                return Err(format!("{:?}", counters(s)));
            }
            if doc_of(s)?.is_modified() || doc_of(s)?.history.undo_len() != 0 || tray::tray_stickers(s).len() != 2 {
                return Err("the reopened project differs".into());
            }
            Ok(())
        }),
        Step::Shot {
            name: "watch_reopened",
            expect: "3D view: the sphere with the magenta badge (dark ring; its picture is wider, with transparent sides) \
                     where the badge was placed, selected; the Sticker tab scrolled to the tray: the wide badge and the green \
                     star, Watching: doc-assets, 2 pictures from it are in the tray, Read again / Show folder / Stop watching",
        },
        Step::Setup("remember", remember_undo),
    ]);
    // (8) Stop watching.
    steps.extend(tap!("Stop watching", PanelHit::TrayWatchStop));
    steps.extend([Step::Assert("no folder; the stickers stay; no entry; modified", |s, sc| {
        if s.doors.watch.is_some() || tray::tray_stickers(s).len() != 2 || !doc_of(s)?.is_modified() {
            return Err("Stop watching did not keep the stickers".into());
        }
        panel_rect(s, PanelHit::TrayWatchFolder)?;
        undo_since(s, sc, 0)
    })]);
    steps
}

/// A hash of a stamp's stored placement.
fn placement_of(s: &AppState, stamp: StampId) -> Result<u64, String> {
    let p = &doc_of(s)?.stamp(stamp).ok_or("no stamp")?.placement;
    Ok(wte_core::fnv1a64(0xcbf2_9ce4_8422_2325, format!("{p:?}").as_bytes()))
}
