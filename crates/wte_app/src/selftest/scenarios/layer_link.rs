// Stickers P12 (STICKER_PLAN 7 "P12", 5.9, 13.9 "P12", 8.2; ARCHITECTURE.md 8.11, 8.16): a linked texture paint layer
// in the real window, on a project saved into the run folder, with the stub launcher (nothing is started) and files the
// scenario writes the way an editor's save lands (temp file + rename).
//
// - `layer_link`: (1) paint "Paint 1" of the sphere, save the project, right-click its row: "Edit Layer in PhotoCraft"
//   -> a one-layer PSD at the texture's size in `<run>_links\`, the stub saw it, the Layers tab shows the link block
//   ("Link: linked"); (2) the editor saves a blue band -> one undo step "Update Linked Layer", the layer and the model
//   update; (3) undo, 2 s: nothing comes back by itself, the badge says "changed in WTE"; "Update now" (the block's
//   button) applies it again; (4) WTE paints and the editor saves a green band -> the conflict question (never
//   automatic), "Take PhotoCraft's version"; (5) save and reopen: the link is back, "linked", the pixels the same.
//   Then once with the stub as **Photoshop** (13.9): Photoshop found through an injected registry and made the default
//   editor; "Paint 2" painted and linked from the Layer menu's "Edit Layer in Photoshop" -> the stub saw Photoshop;
//   Photoshop's save -> "Updated from Photoshop"; a canvas resized in Photoshop -> nothing applied, the sentence "...
//   in Photoshop; the texture is ... Undo the size change in Photoshop." on the layer badge; two layers saved without
//   a merged picture -> "needs a merged picture" with Photoshop's MaxCompat sentence.
// Each shot says what it must show.

use super::*;
use crate::link::{EditRequest, LinkAction, LinkId, LinkTarget, QuestionKind, testkit};
use wte_core::LayerId;

fn project(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, ".wtep")
}

fn layer_key(s: &AppState, sc: &Scratch, key: &str) -> Result<LayerId, String> {
    let id = LayerId(remembered(sc, key)?);
    doc_of(s)?.layer(id).ok_or_else(|| format!("the layer \"{key}\" is gone"))?;
    Ok(id)
}

fn link_file(s: &AppState, id: LayerId) -> Result<PathBuf, String> {
    crate::link::link_path(s, id).ok_or_else(|| "the layer has no link file".to_owned())
}

fn texture(s: &AppState, id: LayerId) -> Result<[u32; 2], String> {
    doc_of(s)?.layer_texture_size(id).ok_or_else(|| "no texture".to_owned())
}

fn dense(s: &AppState, id: LayerId) -> Result<PixelBuf, String> {
    let [w, h] = texture(s, id)?;
    Ok(doc_of(s)?.layer(id).ok_or("no layer")?.to_dense(w, h))
}

/// The badge of layer `key` is `want` (a Setup: the badge needs the document mutably for its CRC cache).
fn badge_is(s: &mut AppState, sc: &Scratch, key: &str, want: &str) -> Result<(), String> {
    let id = layer_key(s, sc, key)?;
    let got = crate::link::badge(s, id).map(|(_, t)| t).ok_or("not linked")?;
    if got == want { Ok(()) } else { Err(format!("the badge reads \"{got}\", expected \"{want}\"")) }
}

fn entry_is(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

/// Remembers the update counter, the history length and the composite.
fn remember_link(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    sc.values.insert("applied", s.link_ui.counters.applied);
    sc.values.insert("composite", composite_hash(doc_of(s)?));
    remember_undo(s, sc)
}

fn one_more_update(s: &AppState, sc: &Scratch) -> Result<bool, String> {
    Ok(s.link_ui.counters.applied == remembered(sc, "applied")? + 1)
}

/// The picture in layer `key`'s file with a band of rows `y0..y1` (fractions of the height) filled with `rgba`; the
/// editor saves it (one layer, as a plain repaint in the editor would).
fn editor_paints_band(s: &AppState, sc: &mut Scratch, key: &str, band: [f32; 2], rgba: [u8; 4]) -> Result<(), String> {
    let id = layer_key(s, sc, key)?;
    let path = link_file(s, id)?;
    let [w, h] = texture(s, id)?;
    let mut picture = wte_core::link::paint_source(&testkit::read(&path)?, [w, h], "photocraft").map_err(|e| format!("{e:?}"))?.pixels;
    for y in (band[0] * h as f32) as u32..(band[1] * h as f32) as u32 {
        for x in 0..w {
            picture.set(x, y, rgba);
        }
    }
    sc.values.insert("file_crc", u64::from(crc_of(&picture)));
    testkit::save_as_editor(&path, &wte_core::link::paint::layer_psd("Paint", &picture, w, h))
}

fn crc_of(p: &PixelBuf) -> u32 {
    wte_core::link::paint::normalised_crc(p, p.width, p.height)
}

/// Layer `key` holds the picture `editor_paints_band` saved last.
fn holds_the_file(s: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    let p = dense(s, layer_key(s, sc, key)?)?;
    if u64::from(crc_of(&p)) == remembered(sc, "file_crc")? { Ok(()) } else { Err("the layer does not hold the file's picture".into()) }
}

fn press(s: &mut AppState, label: &str) -> Result<(), String> {
    crate::link::press(s, label)
}

/// The registry of the Photoshop pass: Photoshop's App Paths key names the stub program in the run folder.
struct StubPhotoshopRegistry(String);

impl crate::link::launch::RegistryReader for StubPhotoshopRegistry {
    fn default_value(&self, hive: crate::link::launch::Hive, view: crate::link::launch::RegView, subkey: &str) -> Option<String> {
        let wanted = hive == crate::link::launch::Hive::LocalMachine
            && view == crate::link::launch::RegView::Bits64
            && subkey == r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Photoshop.exe";
        wanted.then(|| self.0.clone())
    }
}

fn click_hit(s: &AppState, hit: PanelHit, button: egui::PointerButton) -> Result<Vec<Step>, String> {
    let rect = panel_rect(s, hit)?;
    Ok(vec![Step::ScreenClick { at: rect.center(), button, mods: egui::Modifiers::NONE }])
}

pub(super) fn layer_link() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("the stub PhotoCraft program in the run folder; no links folder yet; the Layers tab", |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("photocraft.exe");
            std::fs::create_dir_all(exe.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.prefs.photocraft_exe = Some(exe);
            s.prefs.default_editor = None;
            let links = wte_core::link::links_folder(&project(sc));
            if links.exists() {
                std::fs::remove_dir_all(&links).map_err(|e| e.to_string())?;
            }
            s.side_tab = crate::state::SideTab::Layers;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("a red stroke across the sphere on Paint 1", |s, _| {
            model_stroke(s, ViewId::Main3D, &[[-0.6, -0.3], [-0.2, -0.36], [0.25, -0.3], [0.6, -0.12]])
        }),
        Step::WaitIdle,
        Step::Setup("remember Paint 1 (the active paint layer)", |s, sc| {
            let session = session_of(s)?;
            let layer = session.doc.set(session.active_set).and_then(|x| x.active_layer()).ok_or("no active layer")?;
            if layer.name != "Paint 1" || !layer.is_raster() {
                return Err(format!("the active layer is {}", layer.name));
            }
            sc.values.insert("paint", layer.id.0);
            Ok(())
        }),
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
        // (1) The row menu's "Edit Layer in PhotoCraft".
        Step::WaitFrames(3),
        Step::Dyn("right-click the row of Paint 1", |s, sc| click_hit(s, PanelHit::LayerRow(layer_key(s, sc, "paint")?), egui::PointerButton::Secondary)),
        Step::WaitFrames(3),
        Step::Assert("the row menu offers \"Edit Layer in PhotoCraft\"", |s, sc| {
            panel_rect(s, PanelHit::LayerMenuEditIn)?;
            let label = crate::link::ui::edit_layer_label(s, layer_key(s, sc, "paint")?);
            if label == "Edit Layer in PhotoCraft" { Ok(()) } else { Err(format!("the item reads \"{label}\"")) }
        }),
        Step::Shot {
            name: "layer_link_menu",
            expect: "the context menu of the row \"Paint 1\" in the Layers tab with \"Edit Layer in PhotoCraft\", the submenu \
                     \"Edit Layer in\", \"Export this layer...\"; the sphere with a red stroke",
        },
        Step::Setup("remember the history", remember_undo),
        Step::Dyn("click Edit Layer in PhotoCraft", |s, _| click_hit(s, PanelHit::LayerMenuEditIn, egui::PointerButton::Primary)),
        Step::WaitIdle,
        Step::Setup("the link: a one-layer PSD at the texture's size in the links folder; the stub saw it; no new entry", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            let path = link_file(s, id)?;
            if path.parent() != Some(wte_core::link::links_folder(&project(sc)).as_path()) {
                return Err(format!("the file is at {}", path.display()));
            }
            let read = testkit::read(&path)?;
            if read.layers.len() != 1 || read.size != texture(s, id)? {
                return Err(format!("{} layers, {:?}", read.layers.len(), read.size));
            }
            let calls = s.link_ui.launcher.calls();
            if calls.len() != 1 || calls[0].file != path || calls[0].editor != "photocraft" {
                return Err(format!("stub calls {calls:?}"));
            }
            undo_since(s, sc, 0)?;
            badge_is(s, sc, "paint", "linked")
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "layer_link_made",
            expect: "Layers tab: at the top of the list, above the rows, the link block \"Link: linked\", \"Editor: PhotoCraft \
                     \u{b7} Paint 1_l<id>.psd\" and the buttons Edit Layer in PhotoCraft, Edit in..., Update now, Send to PhotoCraft, \
                     Show file in Explorer, Unlink; the toast about opacity and blend mode staying in WTE",
        },
        // (2) The editor paints a blue band and saves: one update.
        Step::Setup("remember; the editor saves a blue band", |s, sc| {
            remember_link(s, sc)?;
            editor_paints_band(s, sc, "paint", [0.42, 0.58], [30, 90, 230, 255])
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one undo step \"Update Linked Layer\"; the layer holds the file's picture; the model changed", |s, sc| {
            entry_is(s, sc, "Update Linked Layer", 1)?;
            holds_the_file(s, sc, "paint")?;
            changed_since(s, sc, "composite")
        }),
        Step::Shot {
            name: "layer_link_updated",
            expect: "3D view: a blue band around the sphere's middle besides the red stroke (the editor's change); Layers tab: \
                     \"Link: linked\"; toast \"Updated from PhotoCraft\"",
        },
        // (3) Undo; nothing comes back by itself; Update now.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Setup("remember after the undo", remember_link),
        Step::WaitMs(2000),
        Step::Assert("2 s later: no second update, the same picture", |s, sc| {
            if s.link_ui.counters.applied != remembered(sc, "applied")? {
                return Err("an update arrived by itself".into());
            }
            if composite_hash(doc_of(s)?) != remembered(sc, "composite")? { Err("the model changed".into()) } else { Ok(()) }
        }),
        Step::Setup("the badge reads exactly \"changed in WTE\"", |s, sc| badge_is(s, sc, "paint", "changed in WTE")),
        Step::WaitFrames(3),
        Step::Dyn("click Update now in the link block", |s, _| click_hit(s, PanelHit::LayerLinkUpdateNow, egui::PointerButton::Primary)),
        Step::WaitIdle,
        Step::Setup("applied again: the file's picture, linked", |s, sc| {
            if !one_more_update(s, sc)? {
                return Err("Update now did not apply the file".into());
            }
            holds_the_file(s, sc, "paint")?;
            badge_is(s, sc, "paint", "linked")
        }),
        // (4) Both change: the conflict.
        Step::Dyn("WTE paints another stroke on Paint 1", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.5, 0.3], [0.5, 0.35]])),
        Step::WaitIdle,
        Step::Setup("remember; the editor saves a green band", |s, sc| {
            remember_link(s, sc)?;
            editor_paints_band(s, sc, "paint", [0.2, 0.3], [40, 200, 60, 255])
        }),
        Step::Until("the conflict question", |s, _| Ok(s.link_ui.counters.conflicts >= 1)),
        Step::WaitFrames(3),
        Step::Assert("the question, nothing applied", |s, sc| {
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.kind != (QuestionKind::Conflict { saved_over: false }) || q.lines[0] != "Both WTE and PhotoCraft changed Paint 1." {
                return Err(format!("{:?} {:?}", q.kind, q.lines));
            }
            if s.link_ui.counters.applied != remembered(sc, "applied")? { Err("applied without asking".into()) } else { Ok(()) }
        }),
        Step::Shot {
            name: "layer_link_conflict",
            expect: "the question \"Linked layer changed\" / \"Both WTE and PhotoCraft changed Paint 1.\" with Decide later, \
                     Keep WTE's version and Take PhotoCraft's version over the sphere with the red strokes and the blue band",
        },
        Step::Setup("Take PhotoCraft's version", |s, _| press(s, "Take PhotoCraft's version")),
        Step::WaitIdle,
        Step::Assert("the file's picture, one more entry", |s, sc| {
            holds_the_file(s, sc, "paint")?;
            entry_is(s, sc, "Update Linked Layer", 1)
        }),
        // (5) Save and reopen.
        Step::Setup("linked before the save; remember the pixels", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            sc.values.insert("pixels", u64::from(crc_of(&dense(s, id)?)));
            badge_is(s, sc, "paint", "linked")
        }),
        Step::Do(AppAction::Save),
        Step::WaitIdle,
        Step::Assert("the project holds the file's copy", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            wte_core::project::read_link_archive(&project(sc), &wte_core::link::paint_archive_entry(id)).map(|_| ()).map_err(|e| e.to_string())
        }),
        Step::Dyn("open the project again", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Setup("the link came back, in sync, the same pixels", |s, sc| {
            let id = layer_key(s, sc, "paint")?;
            let link = doc_of(s)?.layer_link(id).ok_or("the link is gone")?;
            if link.editor != "photocraft" {
                return Err(format!("editor {}", link.editor));
            }
            if u64::from(crc_of(&dense(s, id)?)) != remembered(sc, "pixels")? {
                return Err("the pixels changed".into());
            }
            s.side_tab = crate::state::SideTab::Layers;
            badge_is(s, sc, "paint", "linked")
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "layer_link_reopened",
            expect: "the reopened sphere with the first red stroke, the blue and the green band (WTE's second stroke is gone: \
                     PhotoCraft's version was taken); Layers tab: Paint 1 active with \"Link: linked\", \"Editor: PhotoCraft \
                     \u{b7} Paint 1_l<id>.psd\" and the link buttons",
        },
    ];
    steps.extend(photoshop_pass());
    steps
}

/// Stickers P12 / 13.9: the layer link once with the stub as Photoshop (see the module docs).
fn photoshop_pass() -> Vec<Step> {
    vec![
        Step::Setup("Photoshop known through the registry and made the default editor; a fresh stub log; a new paint layer", |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("Photoshop.exe");
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.link_ui.registry = Box::new(StubPhotoshopRegistry(exe.display().to_string()));
            s.link_ui.discovered = false;
            s.link_ui.launcher = Box::new(crate::link::launch::StubLauncher::default());
            s.prefs.default_editor = Some(wte_core::link::EDITOR_PHOTOSHOP.to_owned());
            Ok(())
        }),
        Step::Do(AppAction::AddPaintLayer),
        Step::WaitIdle,
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        Step::Dyn("a blue stroke on the new layer", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.4, 0.05], [0.4, 0.1]])),
        Step::WaitIdle,
        Step::Setup("remember the new layer; the Layer menu's item reads \"Edit Layer in Photoshop\"", |s, sc| {
            let session = session_of(s)?;
            let layer = session.doc.set(session.active_set).and_then(|x| x.active_layer()).ok_or("no active layer")?;
            sc.values.insert("ps", layer.id.0);
            crate::link::editor::ensure_discovered(s, false);
            let label = crate::link::ui::edit_layer_label(s, LayerId(remembered(sc, "ps")?));
            if label == "Edit Layer in Photoshop" { Ok(()) } else { Err(format!("the item reads \"{label}\"")) }
        }),
        Step::Dyn("Layer > Edit Layer in Photoshop", |s, sc| {
            Ok(vec![Step::Do(AppAction::Link(LinkAction::EditIn(EditRequest::new(LinkTarget::Layer(layer_key(s, sc, "ps")?)))))])
        }),
        Step::WaitIdle,
        Step::Setup("linked in Photoshop: the stub saw Photoshop's program", |s, sc| {
            let id = layer_key(s, sc, "ps")?;
            let calls = s.link_ui.launcher.calls();
            if calls.len() != 1 || calls[0].editor != "photoshop" || calls[0].file != link_file(s, id)? {
                return Err(format!("stub calls {calls:?}"));
            }
            let opened = s.link_ui.sessions.get(&LinkId::Layer(id)).is_some_and(|x| x.opened_editor.as_deref() == Some("photoshop"));
            if !opened {
                return Err("not opened in Photoshop".into());
            }
            badge_is(s, sc, "ps", "linked")
        }),
        Step::Setup("remember; Photoshop saves a red band", |s, sc| {
            remember_link(s, sc)?;
            editor_paints_band(s, sc, "ps", [0.6, 0.7], [220, 30, 40, 255])
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one update from Photoshop", |s, sc| {
            entry_is(s, sc, "Update Linked Layer", 1)?;
            holds_the_file(s, sc, "ps")?;
            let toasts = s.toasts.visible_labels();
            if toasts.iter().any(|t| t == "Updated from Photoshop") { Ok(()) } else { Err(format!("toasts {toasts:?}")) }
        }),
        // A canvas resized in Photoshop: refused, the sentence on the badge.
        Step::Setup("remember; Photoshop saves the picture on a larger canvas", |s, sc| {
            remember_link(s, sc)?;
            let id = layer_key(s, sc, "ps")?;
            let [w, h] = texture(s, id)?;
            let path = link_file(s, id)?;
            let wider = PixelBuf::filled(w + 64, h, [10, 10, 10, 255]);
            testkit::save_as_editor(&path, &wte_core::link::paint::layer_psd("Paint", &wider, w + 64, h))
        }),
        Step::Until("the watcher saw it", |s, sc| {
            let id = layer_key(s, sc, "ps")?;
            let observed = s.link_ui.sessions.get(&LinkId::Layer(id)).and_then(|x| x.observed).flatten();
            let synced = doc_of(s)?.layer_link(id).ok_or("not linked")?.synced;
            Ok(observed.is_some_and(|o| !o.same_content(&synced)))
        }),
        Step::WaitIdle,
        Step::Setup("nothing applied; Photoshop's size sentence on the badge", |s, sc| {
            if s.link_ui.counters.applied != remembered(sc, "applied")? || composite_hash(doc_of(s)?) != remembered(sc, "composite")? {
                return Err("the resized file was applied".into());
            }
            undo_since(s, sc, 0)?;
            let id = layer_key(s, sc, "ps")?;
            let [w, h] = texture(s, id)?;
            let want = format!("Paint 2 is {} x {h} in Photoshop; the texture is {w} x {h}. Undo the size change in Photoshop.", w + 64);
            badge_is(s, sc, "ps", &want)
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "layer_link_ps_size",
            expect: "Layers tab: Paint 2 active, its link block reads \"Link: Paint 2 is W x H in Photoshop; the texture is w x h. \
                     Undo the size change in Photoshop.\", \"Editor: Photoshop\", Edit Layer in Photoshop and the link buttons; \
                     the same sentence as a toast",
        },
        // Two layers without a merged picture (MaxCompat off): "needs a merged picture" with Photoshop's sentence.
        Step::Setup("Photoshop saves two layers without a merged picture", |s, sc| {
            let id = layer_key(s, sc, "ps")?;
            let [w, h] = texture(s, id)?;
            let path = link_file(s, id)?;
            let mut psd = wte_core::link::paint::layer_psd("Paint", &PixelBuf::filled(w, h, [5, 5, 5, 255]), w, h);
            psd.layers.push(testkit::rect_layer("Layer 1", 2, wte_core::IRect::from_xywh(0, 0, 16, 16), [255, 255, 255, 255]));
            psd.id_seed = 2;
            let mut bytes = wte_core::psd::write_sticker_to_vec(&psd).map_err(|e| e.to_string())?;
            testkit::clear_real_merged(&mut bytes)?;
            testkit::save_bytes(&path, &bytes)
        }),
        Step::Until("the badge says it needs a merged picture", |s, sc| {
            let id = layer_key(s, sc, "ps")?;
            let link = doc_of(s)?.layer_link(id).ok_or("not linked")?;
            Ok(matches!(link.problem, Some(wte_core::link::LinkProblem::NeedsMerged { .. })))
        }),
        Step::Setup("Photoshop's MaxCompat sentence word for word", |s, sc| {
            badge_is(s, sc, "ps", "needs a merged picture")?;
            let toasts = s.toasts.visible_labels();
            let want = "Photoshop saved this file without a merged picture. In Photoshop choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File Compatibility\" to Always, and save again.";
            if toasts.iter().any(|t| t == want) { Ok(()) } else { Err(format!("toasts {toasts:?}")) }
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "layer_link_ps_merged",
            expect: "Layers tab: Paint 2's link block reads \"Link: needs a merged picture\" with Photoshop's sentence (Edit > \
                     Preferences > File Handling ... to Always); the sphere unchanged",
        },
    ]
}
