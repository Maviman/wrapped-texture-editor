// objects-tabs (WP6; OBJECTS_TABS_PLAN 8.1 / 8.2, ARCHITECTURE.md 8.11): the scenarios `objects` (the built-in Three
// Objects sample: select by a Navigate click, hide, paint and stamp through the hidden Cube, show, isolate, frame, the
// list's eye / Ctrl / Shift clicks, the footer, and hidden objects kept by a save and reopen) and `objects_path` (a
// multi-object model given as PATH, e.g. ABeautifulGame.glb: the virtualized list, isolate within the 250 ms frame
// budget, frame, a Navigate pick revealed in the list, a brush stroke on the isolated pieces). All asserts are on state;
// the named shots are for the reviewer.

use super::*;

const FLOOR: u32 = 0;
const CUBE: u32 = 1;
const BALL: u32 = 2;

/// The UI-thread budget of the frame that applies a visibility change on ABeautifulGame (plan 8.2 step 3).
const VISIBILITY_FRAME_MS: f64 = 250.0;
/// Brush budget of the gate reports (ROUND5_GATE_REPORT B4: no frame of stroke work over 140 ms after the warm-up
/// stroke on ABeautifulGame at Size 50).
const BRUSH_FRAME_MS: f64 = 140.0;

// ---------------------------------------------------------------------------------------------------------------------
// objects
// ---------------------------------------------------------------------------------------------------------------------

/// The `objects` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn objects() -> Vec<Step> {
    let steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Objects)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        // (1) The three objects, nothing hidden, the Tool tab.
        Step::Assert("three objects Floor, Cube, Ball; nothing hidden; the Tool tab", |s, _| {
            let doc = doc_of(s)?;
            let names: Vec<&str> = doc.mesh.objects.iter().map(|o| o.name.as_str()).collect();
            if names != ["Floor", "Cube", "Ball"] {
                return Err(format!("the objects are {names:?}"));
            }
            let sum: u64 = doc.mesh.objects.iter().map(|o| u64::from(o.tri_count)).sum();
            if sum != u64::from(doc.mesh.tri_count()) {
                return Err(format!("the objects hold {sum} triangles, the mesh {}", doc.mesh.tri_count()));
            }
            if doc.hidden_object_count() != 0 {
                return Err(format!("hidden at the start: {:?}", doc.hidden_objects()));
            }
            if s.upper_tab != UpperTab::Tool {
                return Err(format!("the upper tab is {:?}", s.upper_tab));
            }
            Ok(())
        }),
        Step::Setup("remember the regions of the composite before any edit", |s, sc| {
            let [floor, cube, ball] = region_hashes(doc_of(s)?)?;
            sc.values.insert("floor_0", floor);
            sc.values.insert("cube_0", cube);
            sc.values.insert("ball_0", ball);
            Ok(())
        }),
        // (2) The Objects tab; a Navigate click on the Cube selects it.
        Step::Setup("the Objects tab", |s, _| {
            s.upper_tab = UpperTab::Objects;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Navigate)),
        Step::WaitFrames(2),
        Step::Dyn("Navigate click on the Cube's centre", |s, _| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let centre = doc_of(s)?.mesh.objects[CUBE as usize].bounds.center();
            let pos = world_screen(&xf, centre)?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, pos))])
        }),
        Step::WaitFrames(3),
        Step::Assert("the Cube is selected and its row drawn", |s, _| {
            selection_is(s, &[CUBE])?;
            panel_rect(s, PanelHit::ObjectRow(CUBE)).map(|_| ())
        }),
        Step::Shot { name: "selected", expect: "" },
        // (3) The ray through that point hits the Cube; the document's revision, modified flag and history.
        Step::Setup("remember the ray through the Cube and the document state", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let centre = doc_of(s)?.mesh.objects[CUBE as usize].bounds.center();
            let pos = world_screen(&xf, centre)?;
            sc.values.insert("cube_pos", pack(Vec2::new(pos.x, pos.y)));
            let hit = object_under(s, sc, "cube_pos")?;
            if hit != Some(CUBE) {
                return Err(format!("the ray through the Cube's centre hits {hit:?}"));
            }
            remember_doc_state(s, sc)
        }),
        // (4) H hides the Cube: the ray reaches the Floor; nothing of the document changed.
        Step::Key(egui::Modifiers::NONE, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("H hid the Cube; the ray now hits the Floor; view state only", |s, sc| {
            hidden_are(s, &[CUBE])?;
            selection_is(s, &[CUBE])?;
            let hit = object_under(s, sc, "cube_pos")?;
            if hit != Some(FLOOR) {
                return Err(format!("with the Cube hidden the ray hits {hit:?}"));
            }
            doc_state_unchanged(s, sc)
        }),
        Step::Do(AppAction::ToggleWireframe),
        Step::WaitFrames(3),
        Step::Shot { name: "hidden", expect: "" },
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitFrames(3),
        Step::Shot { name: "hidden_2d", expect: "" },
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Do(AppAction::ToggleWireframe),
        Step::WaitFrames(3),
        Step::Assert("the wireframe and the 3D view are back, still view state only", |s, sc| {
            if s.view_mode != ViewMode::View3D {
                return Err(format!("the view mode is {:?}", s.view_mode));
            }
            doc_state_unchanged(s, sc)
        }),
        // (5) A brush stroke across the Cube's footprint paints the Floor under it and never the Cube.
        Step::Setup("remember the Floor texels under the Cube", |s, sc| {
            let doc = doc_of(s)?;
            let hit = doc.raycast(&wte_core::Ray::new(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y)).ok_or("the ray down the Cube's axis hits nothing")?;
            if doc.mesh.tri_object_of(hit.tri) != FLOOR {
                return Err(format!("the ray down the Cube's axis hits object {}", doc.mesh.tri_object_of(hit.tri)));
            }
            let uv = hit.uv.ok_or("the Floor has no UV under the Cube")?;
            sc.values.insert("under_uv", pack(uv));
            sc.values.insert("under_0", under_hash(doc, uv)?);
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(RED)),
        Step::Dyn("a stroke over the Floor across the hidden Cube's footprint", |s, _| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let points = [-0.45f32, -0.15, 0.15, 0.45]
                .iter()
                .map(|&x| world_screen(&xf, Vec3::new(x, 0.0, 0.0)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![drag(&xf, &points)])
        }),
        Step::WaitIdle,
        Step::Assert("the stroke painted the Floor under the Cube, not the Cube; one Brush Stroke", |s, sc| {
            let doc = doc_of(s)?;
            let [floor, cube, _] = region_hashes(doc)?;
            if cube != remembered(sc, "cube_0")? {
                return Err("the Cube's UV region changed".to_owned());
            }
            if floor == remembered(sc, "floor_0")? {
                return Err("the Floor's UV region did not change".to_owned());
            }
            if under_hash(doc, unpack(remembered(sc, "under_uv")?))? == remembered(sc, "under_0")? {
                return Err("the Floor texels under the Cube did not change".to_owned());
            }
            undo_since(s, sc, 1)?;
            match doc.undo_label() {
                Some("Brush Stroke") => Ok(()),
                other => Err(format!("the last undo entry is {other:?}")),
            }
        }),
        Step::Setup("remember the composite after the stroke", |s, sc| {
            let [floor, _, _] = region_hashes(doc_of(s)?)?;
            sc.values.insert("floor_1", floor);
            Ok(())
        }),
        // (6) A projected image stamp at the same point lands on the Floor, not on the hidden Cube.
        Step::Setup("register the sample stamp image (Project)", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.mode = SurfaceMode::Project;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::Dyn("place the image stamp at the Cube's point", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let p = unpack(remembered(sc, "cube_pos")?);
            Ok(vec![click(ViewId::Main3D, norm(&xf, egui::pos2(p.x, p.y)))])
        }),
        Step::WaitIdle,
        Step::Assert("the stamp inked the Floor and not the Cube", |s, sc| {
            let doc = doc_of(s)?;
            let (id, _) = placed_stamp(s, false)?;
            let set = &doc.sets[0];
            let ink = ink_per_region(&layer_canvas(ws_layer(doc, id)?, set.width, set.height));
            log::info!("objects: stamp ink per region (Floor, Cube, Ball) {ink:?}");
            if ink[FLOOR as usize] == 0 || ink[CUBE as usize] != 0 {
                return Err(format!("the stamp's ink per region (Floor, Cube, Ball) is {ink:?}"));
            }
            let [floor, cube, _] = region_hashes(doc)?;
            if cube != remembered(sc, "cube_0")? {
                return Err("the Cube's UV region changed".to_owned());
            }
            if floor == remembered(sc, "floor_1")? {
                return Err("the stamp did not change the Floor's region".to_owned());
            }
            Ok(())
        }),
        Step::Setup("remember the Floor after the stamp", |s, sc| {
            let [floor, _, _] = region_hashes(doc_of(s)?)?;
            sc.values.insert("floor_2", floor);
            Ok(())
        }),
        // (7) Alt+H shows everything: the ray hits the Cube again; the floor keeps its paint, the cube is clean.
        Step::Key(egui::Modifiers::ALT, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("Alt+H showed the Cube; the paint stays on the Floor, the Cube is clean", |s, sc| {
            hidden_are(s, &[])?;
            let hit = object_under(s, sc, "cube_pos")?;
            if hit != Some(CUBE) {
                return Err(format!("with everything shown the ray hits {hit:?}"));
            }
            let [floor, cube, _] = region_hashes(doc_of(s)?)?;
            if cube != remembered(sc, "cube_0")? || floor != remembered(sc, "floor_2")? {
                return Err("showing the Cube changed the composite".to_owned());
            }
            Ok(())
        }),
        Step::Shot { name: "shown", expect: "" },
        // (8) A click on the Ball's row, Shift+H isolates it, Shift+F frames it.
        Step::Setup("the Objects tab", |s, _| {
            s.upper_tab = UpperTab::Objects;
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Dyn("click the Ball's row", |s, _| hit_click(s, PanelHit::ObjectRow(BALL), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Assert("the Ball is selected", |s, _| selection_is(s, &[BALL])),
        Step::Setup("remember the camera distance", |s, sc| {
            sc.values.insert("distance", u64::from(session_of(s)?.camera.distance.to_bits()));
            Ok(())
        }),
        Step::Key(egui::Modifiers::SHIFT, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("Shift+H isolated the Ball", |s, _| hidden_are(s, &[FLOOR, CUBE])),
        Step::Key(egui::Modifiers::SHIFT, egui::Key::F),
        Step::WaitFrames(3),
        Step::Assert("Shift+F framed the Ball", |s, sc| {
            let session = session_of(s)?;
            let ball = session.doc.mesh.objects[BALL as usize].bounds.center();
            let off = (session.camera.target - ball).length();
            let before = f32::from_bits(remembered(sc, "distance")? as u32);
            log::info!("objects: framed the Ball: target off by {off}, distance {before} -> {}", session.camera.distance);
            if off > 1e-3 * session.doc.mesh.diag() {
                return Err(format!("the camera target {:?} is {off} from the Ball's centre {ball:?}", session.camera.target));
            }
            if session.camera.distance >= before {
                return Err(format!("the camera did not come closer: {before} -> {}", session.camera.distance));
            }
            Ok(())
        }),
        Step::Shot { name: "isolated", expect: "" },
        // (9) The Floor's eye shows the Floor; the selection stays.
        Step::Dyn("click the Floor's eye", |s, _| hit_click(s, PanelHit::ObjectEye(FLOOR), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Assert("the eye showed the Floor and kept the selection", |s, _| {
            hidden_are(s, &[CUBE])?;
            selection_is(s, &[BALL])
        }),
        // (10) Ctrl-click the Cube, Shift-click the Floor, the footer's Show all.
        Step::Dyn("Ctrl-click the Cube's row", |s, _| hit_click(s, PanelHit::ObjectRow(CUBE), egui::Modifiers::COMMAND)),
        Step::WaitFrames(3),
        Step::Assert("Ctrl added the Cube", |s, _| selection_is(s, &[CUBE, BALL])),
        Step::Dyn("Shift-click the Floor's row", |s, _| hit_click(s, PanelHit::ObjectRow(FLOOR), egui::Modifiers::SHIFT)),
        Step::WaitFrames(3),
        Step::Assert("Shift selected the range from the Cube to the Floor", |s, _| selection_is(s, &[FLOOR, CUBE])),
        Step::Dyn("click the footer's Show all", |s, _| hit_click(s, PanelHit::ObjectsShowAll, egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Assert("Show all showed everything and kept the selection", |s, _| {
            hidden_are(s, &[])?;
            selection_is(s, &[FLOOR, CUBE])
        }),
        // (11) The Cube hidden again survives a save and reopen; the selection does not.
        Step::Dyn("click the Cube's row", |s, _| hit_click(s, PanelHit::ObjectRow(CUBE), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Key(egui::Modifiers::NONE, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("the Cube alone is hidden", |s, _| hidden_are(s, &[CUBE])),
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            let session = session_of(s)?;
            if session.doc.is_modified() {
                return Err("the document is still modified after the save".to_owned());
            }
            file_non_empty(&with_suffix(&sc.out_stem, ".wtep"))
        }),
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the reopened project hides the Cube, says so once, is not modified, has no selection", |s, sc| {
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            hidden_are(s, &[CUBE])?;
            selection_is(s, &[])?;
            if session.doc.is_modified() {
                return Err("the reopened document is modified".to_owned());
            }
            let texts = s.toasts.visible_labels();
            let hidden: Vec<&String> = texts.iter().filter(|t| t.contains("hidden")).collect();
            if hidden.len() != 1 || !hidden[0].contains("Cube") {
                return Err(format!("the toasts about hidden objects are {hidden:?} (all: {texts:?})"));
            }
            log::info!("objects: reopened with the toast {:?}", hidden[0]);
            Ok(())
        }),
        // (12) The layout with the Objects tab for the final screenshot.
        Step::Setup("the Objects tab for the final screenshot", |s, _| {
            s.upper_tab = UpperTab::Objects;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("the side panel layout", assert_side_areas_apart),
    ];
    steps
}

/// The object under the remembered screen point `key` of the 3D view (through the same masked ray the tools use).
fn object_under(s: &AppState, sc: &Scratch, key: &str) -> Result<Option<u32>, String> {
    let xf = view_xf(s, ViewId::Main3D)?;
    let doc = doc_of(s)?;
    let p = unpack(remembered(sc, key)?);
    Ok(match xf.surface_at(doc, TextureSetId(0), egui::pos2(p.x, p.y)) {
        Surface::Model(hit) => Some(doc.mesh.tri_object_of(hit.tri)),
        _ => None,
    })
}

fn selection_is(s: &AppState, want: &[u32]) -> Result<(), String> {
    let got: Vec<u32> = session_of(s)?.selected_objects.iter().copied().collect();
    if got == want { Ok(()) } else { Err(format!("the selected objects are {got:?}, expected {want:?}")) }
}

/// Exactly the objects `want` are hidden.
fn hidden_are(s: &AppState, want: &[u32]) -> Result<(), String> {
    let doc = doc_of(s)?;
    let got: Vec<u32> = (0..doc.mesh.object_count()).filter(|&o| doc.object_hidden(o)).collect();
    if got == want { Ok(()) } else { Err(format!("the hidden objects are {got:?}, expected {want:?}")) }
}

fn remember_doc_state(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let doc = doc_of(s)?;
    sc.values.insert("revision", doc.revision());
    sc.values.insert("modified", u64::from(doc.is_modified()));
    sc.values.insert("undo_view", doc.history.undo_len() as u64);
    Ok(())
}

/// Hiding is view state: no revision change, the modified flag and the history as remembered.
fn doc_state_unchanged(s: &AppState, sc: &Scratch) -> Result<(), String> {
    let doc = doc_of(s)?;
    let now = [doc.revision(), u64::from(doc.is_modified()), doc.history.undo_len() as u64];
    let then = [remembered(sc, "revision")?, remembered(sc, "modified")?, remembered(sc, "undo_view")?];
    if now != then { Err(format!("revision / modified / undo depth {now:?}, before {then:?}")) } else { Ok(()) }
}

/// The region of a texel of the sample's texture: 0 Floor (u >= 0.5), 1 Cube (u < 0.5, v < 0.5), 2 Ball.
fn region(x: u32, y: u32, width: u32, height: u32) -> usize {
    if x >= width / 2 {
        FLOOR as usize
    } else if y < height / 2 {
        CUBE as usize
    } else {
        BALL as usize
    }
}

/// Hashes of set 0's composite per region (Floor, Cube, Ball).
fn region_hashes(doc: &Document) -> Result<[u64; 3], String> {
    let img = doc.sets.first().ok_or("no texture set")?.composite();
    let mut h = [0xcbf2_9ce4_8422_2325u64; 3];
    for y in 0..img.height {
        for x in 0..img.width {
            let r = region(x, y, img.width, img.height);
            h[r] = wte_core::fnv1a64(h[r], &img.get(x, y));
        }
    }
    Ok(h)
}

/// Texels with ink per region (Floor, Cube, Ball).
fn ink_per_region(img: &PixelBuf) -> [usize; 3] {
    let mut out = [0usize; 3];
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] > 0 {
                out[region(x, y, img.width, img.height)] += 1;
            }
        }
    }
    out
}

/// Hash of the composite's 7 x 7 texels around `uv`.
fn under_hash(doc: &Document, uv: Vec2) -> Result<u64, String> {
    let img = doc.sets.first().ok_or("no texture set")?.composite();
    let (cx, cy) = ((uv.x * img.width as f32) as i64, (uv.y * img.height as f32) as i64);
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for y in cy - 3..=cy + 3 {
        for x in cx - 3..=cx + 3 {
            if x >= 0 && y >= 0 && (x as u32) < img.width && (y as u32) < img.height {
                h = wte_core::fnv1a64(h, &img.get(x as u32, y as u32));
            }
        }
    }
    Ok(h)
}

/// A click on a recorded panel control with modifiers.
fn hit_click(s: &AppState, hit: PanelHit, mods: egui::Modifiers) -> Result<Vec<Step>, String> {
    let rect = panel_rect(s, hit)?;
    Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods }])
}

// ---------------------------------------------------------------------------------------------------------------------
// objects_path
// ---------------------------------------------------------------------------------------------------------------------

/// The `objects_path` steps: PATH is a model with several objects.
pub(super) fn objects_path(has_path: bool) -> Vec<Step> {
    if !has_path {
        return vec![Step::Assert("a PATH argument was given", |_, _| {
            Err("the objects_path scenario needs a model with several objects as PATH (e.g. ABeautifulGame.glb)".to_owned())
        })];
    }
    let mut steps = open_and_accept(true);
    steps.extend([
        Step::WaitFrames(3),
        // (1) Several objects with unique, non-empty names whose triangles add up to the mesh.
        Step::Assert("several objects, unique non-empty names, counts add up", |s, _| {
            let doc = doc_of(s)?;
            let objects = &doc.mesh.objects;
            if objects.len() < 2 {
                return Err(format!(
                    "the model has {} object(s) ({:?}); objects_path needs a model with several objects",
                    objects.len(),
                    objects.iter().map(|o| o.name.as_str()).collect::<Vec<_>>()
                ));
            }
            let names: std::collections::BTreeSet<&str> = objects.iter().map(|o| o.name.as_str()).collect();
            if names.len() != objects.len() || names.contains("") {
                return Err("the object names are not unique and non-empty".to_owned());
            }
            let sum: u64 = objects.iter().map(|o| u64::from(o.tri_count)).sum();
            if sum != u64::from(doc.mesh.tri_count()) {
                return Err(format!("the objects hold {sum} triangles, the mesh {}", doc.mesh.tri_count()));
            }
            log::info!("objects_path: {} objects, {} triangles", objects.len(), doc.mesh.tri_count());
            Ok(())
        }),
        // (2) The Objects tab lays out only the rows in view.
        Step::Setup("the Objects tab", |s, _| {
            s.upper_tab = UpperTab::Objects;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("only the rows in view are laid out", |s, _| {
            let list = panel_rect(s, PanelHit::ObjectsList)?;
            let rows = drawn_rows(s);
            let most = (list.height() / crate::panels::objects::ROW_HEIGHT) as usize + 2;
            log::info!("objects_path: {} rows laid out in a list {:.0} pt high (at most {most})", rows.len(), list.height());
            if rows.is_empty() || rows.len() > most {
                return Err(format!("{} rows laid out, at most {most} fit", rows.len()));
            }
            panel_rect(s, PanelHit::ObjectsFooter).map(|_| ())
        }),
        // (3) Click the first row, Ctrl-click the last object (revealed first), isolate both within the budget, frame.
        Step::Dyn("click the first row", |s, _| hit_click(s, PanelHit::ObjectRow(0), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Setup("reveal the last row", |s, _| {
            let last = last_object(s)?;
            s.session.as_mut().ok_or("no session")?.object_reveal = Some(last);
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Dyn("Ctrl-click the last row", |s, _| hit_click(s, PanelHit::ObjectRow(last_object(s)?), egui::Modifiers::COMMAND)),
        Step::WaitFrames(3),
        Step::Assert("the first and the last object are selected", |s, _| {
            let want = [0, last_object(s)?];
            selection_is(s, &want)
        }),
        Step::Setup("reset the frame clock", |s, _| {
            s.frame_work.reset();
            Ok(())
        }),
        Step::Key(egui::Modifiers::SHIFT, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("Shift+H isolated the two within the frame budget", |s, _| {
            let want = [0, last_object(s)?];
            let doc = doc_of(s)?;
            let visible: Vec<u32> = (0..doc.mesh.object_count()).filter(|&o| !doc.object_hidden(o)).collect();
            if visible != want {
                return Err(format!("the visible objects are {visible:?}, expected {want:?}"));
            }
            let ms = s.frame_work.max.as_secs_f64() * 1000.0;
            log::info!("objects_path: isolate: longest frame {ms:.1} ms over {} frames (budget {VISIBILITY_FRAME_MS} ms)", s.frame_work.frames);
            if ms >= VISIBILITY_FRAME_MS {
                return Err(format!("the visibility change took a frame of {ms:.0} ms (budget {VISIBILITY_FRAME_MS} ms)"));
            }
            Ok(())
        }),
        Step::Setup("remember the camera distance", |s, sc| {
            sc.values.insert("distance", u64::from(session_of(s)?.camera.distance.to_bits()));
            Ok(())
        }),
        Step::Key(egui::Modifiers::SHIFT, egui::Key::F),
        Step::WaitFrames(3),
        Step::Assert("Shift+F framed the selection", |s, sc| {
            let session = session_of(s)?;
            let bounds = crate::object_ops::selection_bounds(session);
            let off = (session.camera.target - bounds.center()).length();
            let before = f32::from_bits(remembered(sc, "distance")? as u32);
            log::info!("objects_path: framed the selection: target off by {off}, distance {before} -> {}", session.camera.distance);
            if off > 1e-3 * session.doc.mesh.diag() {
                return Err(format!("the camera target {:?} is {off} from the selection's centre", session.camera.target));
            }
            Ok(())
        }),
        Step::Shot { name: "isolated", expect: "" },
        // (4) Scroll the list back to the top, then a Navigate click on the last object selects it and reveals its row.
        Step::Setup("scroll the list to the top", |s, _| {
            s.session.as_mut().ok_or("no session")?.object_reveal = Some(0);
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Navigate)),
        Step::WaitFrames(3),
        Step::Assert("the last row is out of view (when the list scrolls)", |s, _| {
            let last = last_object(s)?;
            let list = panel_rect(s, PanelHit::ObjectsList)?;
            // A short list (Lantern: 3 objects) shows every row: nothing to reveal, the pick is still checked.
            if (last as f32 + 1.0) * crate::panels::objects::ROW_HEIGHT <= list.height() {
                log::info!("objects_path: all {} rows fit in the list; the reveal is not tested", last + 1);
                return Ok(());
            }
            match s.panel_rects.get(&PanelHit::ObjectRow(last)) {
                Some(r) if list.contains_rect(*r) => Err(format!("the last row {r:?} is still inside the list {list:?}")),
                _ => Ok(()),
            }
        }),
        Step::Dyn("Navigate click on the last object", |s, _| {
            let last = last_object(s)?;
            let (xf, hits) = model_hits(s)?;
            let doc = doc_of(s)?;
            let on_last = |h: &&ScreenHit| match xf.surface_at(doc, TextureSetId(0), h.pos) {
                Surface::Model(hit) => doc.mesh.tri_object_of(hit.tri) == last,
                _ => false,
            };
            let ranked = best_spots(&hits);
            let spot = ranked.iter().find(on_last).ok_or("no grid point of the view hits the last object")?;
            Ok(vec![click(ViewId::Main3D, norm(&xf, spot.pos))])
        }),
        Step::WaitFrames(4),
        Step::Assert("the pick selected the last object and revealed its row", |s, _| {
            let last = last_object(s)?;
            selection_is(s, &[last])?;
            let list = panel_rect(s, PanelHit::ObjectsList)?;
            let row = panel_rect(s, PanelHit::ObjectRow(last))?;
            if !list.expand(0.5).contains_rect(row) {
                return Err(format!("the row {row:?} is not inside the list {list:?}"));
            }
            Ok(())
        }),
        // (5) Brush strokes across the isolated pieces: one undo entry each; the second within the brush budget.
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::SetPrimaryColor(GREEN)),
        Step::Setup("remember the history", remember_undo),
        Step::Dyn("a warm-up stroke across the isolated pieces", |s, _| hit_stroke(s, 0)),
        Step::WaitIdle,
        Step::Assert("the warm-up stroke made one undo entry", |s, sc| undo_since(s, sc, 1)),
        Step::Setup("remember the history and reset the frame clock", |s, sc| {
            s.frame_work.reset();
            remember_undo(s, sc)
        }),
        Step::Dyn("a stroke across the isolated pieces", |s, _| hit_stroke(s, 1)),
        Step::WaitIdle,
        Step::Assert("the stroke made one undo entry within the brush budget", |s, sc| {
            undo_since(s, sc, 1)?;
            let ms = s.frame_work.max.as_secs_f64() * 1000.0;
            log::info!("objects_path: stroke: longest frame {ms:.1} ms over {} frames (budget {BRUSH_FRAME_MS} ms)", s.frame_work.frames);
            if ms > BRUSH_FRAME_MS {
                return Err(format!("a frame of the stroke took {ms:.0} ms (budget {BRUSH_FRAME_MS} ms)"));
            }
            Ok(())
        }),
        // (6) Show All.
        Step::Key(egui::Modifiers::ALT, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("Alt+H showed everything", |s, _| hidden_are(s, &[])),
    ]);
    steps
}

/// The index of the model's last object.
fn last_object(s: &AppState) -> Result<u32, String> {
    doc_of(s)?.mesh.object_count().checked_sub(1).ok_or_else(|| "the model has no object".to_owned())
}

/// The object rows laid out in the last frame.
fn drawn_rows(s: &AppState) -> Vec<u32> {
    let mut rows: Vec<u32> = s.panel_rects.keys().filter_map(|k| if let PanelHit::ObjectRow(i) = k { Some(*i) } else { None }).collect();
    rows.sort_unstable();
    rows
}

// ---------------------------------------------------------------------------------------------------------------------
// objects_flow_path (completeness critic of the gate)
// ---------------------------------------------------------------------------------------------------------------------

/// The `objects_flow_path` steps: the user's whole workflow on a multi-object model given as PATH (ABeautifulGame.glb):
/// hide the board (the object with the largest ground footprint) from its row, select and frame a piece (the first
/// "pawn", else the largest other object), a Navigate pick on the piece and a miss where only the hidden board is, a
/// brush stroke from the piece out over the hidden board, a projected image stamp, a wrapped stamp with "jump small
/// gaps" on and a text on the piece, a Preview shot with the selection, save, reopen (the board hidden again, its rays
/// still pass through), export one layer and every set's flattened PNG, Show all. Every step asserts that the texture
/// sets only the board uses never change and that no stamp or text layer of those sets gets ink.
pub(super) fn objects_flow_path(has_path: bool) -> Vec<Step> {
    if !has_path {
        return vec![Step::Assert("a PATH argument was given", |_, _| {
            Err("the objects_flow_path scenario needs a model with several objects as PATH (e.g. ABeautifulGame.glb)".to_owned())
        })];
    }
    let mut steps = open_and_accept(true);
    steps.extend([
        Step::WaitFrames(3),
        // (1) The board and the piece; the texture sets only the board uses must exist for the checks to mean anything.
        Step::Setup("choose the board and the piece", |s, sc| {
            let doc = doc_of(s)?;
            let (big, piece) = flow_choose(doc)?;
            let sets = object_sets(doc);
            let board = board_only_sets(&sets, big);
            log::info!(
                "objects_flow_path: board {:?} (sets {:?}, only the board: {board:?}), piece {:?} (sets {:?})",
                doc.mesh.objects[big as usize].name,
                sets[big as usize],
                doc.mesh.objects[piece as usize].name,
                sets[piece as usize]
            );
            if board.is_empty() {
                return Err("no texture set is used by the board alone: the board checks would prove nothing".to_owned());
            }
            sc.values.insert("big", u64::from(big));
            sc.values.insert("piece", u64::from(piece));
            sc.values.insert("board_sets", sets_hash(doc, &board)?);
            let (on_big, total) = board_ray_hits(doc, big);
            log::info!("objects_flow_path: rays down over the board: {on_big} of {total} hits on the board (all shown)");
            if on_big == 0 {
                return Err("no ray down over the board's footprint hits the board".to_owned());
            }
            Ok(())
        }),
        // (2) The Objects tab; the board's row (revealed first), H.
        Step::Setup("the Objects tab, the board's row revealed", |s, sc| {
            s.upper_tab = UpperTab::Objects;
            s.session.as_mut().ok_or("no session")?.object_reveal = Some(remembered(sc, "big")? as u32);
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Dyn("click the board's row", |s, sc| hit_click(s, PanelHit::ObjectRow(remembered(sc, "big")? as u32), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Assert("the board is selected", |s, sc| selection_is(s, &[remembered(sc, "big")? as u32])),
        Step::Setup("remember the document state", remember_doc_state),
        Step::Key(egui::Modifiers::NONE, egui::Key::H),
        Step::WaitFrames(3),
        Step::Assert("H hid the board: rays pass through it, view state only", |s, sc| {
            let big = remembered(sc, "big")? as u32;
            hidden_are(s, &[big])?;
            doc_state_unchanged(s, sc)?;
            let (on_big, total) = board_ray_hits(doc_of(s)?, big);
            log::info!("objects_flow_path: board hidden: {on_big} of {total} ray hits on the board");
            if on_big != 0 { Err(format!("{on_big} rays down still hit the hidden board")) } else { Ok(()) }
        }),
        Step::Shot { name: "board_hidden", expect: "" },
        // (3) The piece: its row (revealed), Shift+F.
        Step::Setup("reveal the piece's row", |s, sc| {
            s.session.as_mut().ok_or("no session")?.object_reveal = Some(remembered(sc, "piece")? as u32);
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Dyn("click the piece's row", |s, sc| hit_click(s, PanelHit::ObjectRow(remembered(sc, "piece")? as u32), egui::Modifiers::NONE)),
        Step::WaitFrames(3),
        Step::Key(egui::Modifiers::SHIFT, egui::Key::F),
        Step::WaitFrames(3),
        Step::Assert("the piece is selected and framed; the board still hidden", |s, sc| {
            let (big, piece) = (remembered(sc, "big")? as u32, remembered(sc, "piece")? as u32);
            selection_is(s, &[piece])?;
            hidden_are(s, &[big])?;
            let session = session_of(s)?;
            let off = (session.camera.target - session.doc.mesh.objects[piece as usize].bounds.center()).length();
            if off > 1e-3 * session.doc.mesh.diag() {
                return Err(format!("the camera target is {off} from the piece's centre"));
            }
            Ok(())
        }),
        // (4) Where the stroke goes: from the piece's centre out to a point where only the hidden board is.
        Step::Setup("plan the stroke from the piece over the hidden board", |s, sc| {
            let (xf, c, e) = flow_stroke_points(s, sc)?;
            sc.values.insert("flow_c", pack(Vec2::new(c.x, c.y)));
            sc.values.insert("flow_e", pack(Vec2::new(e.x, e.y)));
            log::info!("objects_flow_path: stroke {c:?} -> {e:?} in the view {:?}", xf.rect);
            Ok(())
        }),
        // (5) Navigate: a click where only the hidden board is deselects, a click on the piece selects it.
        Step::Do(AppAction::SetTool(ToolKind::Navigate)),
        Step::WaitFrames(2),
        Step::Dyn("Navigate click where only the hidden board is", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let e = unpack(remembered(sc, "flow_e")?);
            Ok(vec![click(ViewId::Main3D, norm(&xf, egui::pos2(e.x, e.y)))])
        }),
        Step::WaitFrames(3),
        Step::Assert("the click through the hidden board selected nothing", |s, _| selection_is(s, &[])),
        Step::Dyn("Navigate click on the piece", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let c = unpack(remembered(sc, "flow_c")?);
            Ok(vec![click(ViewId::Main3D, norm(&xf, egui::pos2(c.x, c.y)))])
        }),
        Step::WaitFrames(3),
        Step::Assert("the pick selected the piece", |s, sc| selection_is(s, &[remembered(sc, "piece")? as u32])),
        Step::Shot { name: "piece_selected", expect: "" },
        // (6) The brush stroke: the piece's sets change, the board's do not; one undo entry.
        Step::Setup("remember the piece's sets and the history", |s, sc| {
            let doc = doc_of(s)?;
            let piece = remembered(sc, "piece")? as u32;
            let sets: Vec<u32> = object_sets(doc)[piece as usize].iter().copied().collect();
            sc.values.insert("piece_sets", sets_hash(doc, &sets)?);
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Do(AppAction::BrushSizeStep(2)),
        Step::Do(AppAction::SetPrimaryColor(GREEN)),
        Step::Dyn("a stroke from the piece out over the hidden board", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let (c, e) = (unpack(remembered(sc, "flow_c")?), unpack(remembered(sc, "flow_e")?));
            Ok(vec![drag(&xf, &[egui::pos2(c.x, c.y), egui::pos2(e.x, e.y)])])
        }),
        Step::WaitIdle,
        Step::Assert("the stroke painted the piece and not the hidden board; one undo entry", |s, sc| {
            let doc = doc_of(s)?;
            let piece = remembered(sc, "piece")? as u32;
            let sets: Vec<u32> = object_sets(doc)[piece as usize].iter().copied().collect();
            if sets_hash(doc, &sets)? == remembered(sc, "piece_sets")? {
                return Err("the stroke did not change the piece's texture sets".to_owned());
            }
            board_untouched(s, sc)?;
            undo_since(s, sc, 1)
        }),
        // (7) A projected image stamp on the piece, bigger than the piece.
        Step::Setup("the sample stamp image, Project, larger than the piece", |s, sc| {
            setup_stamp_image(s, sc)?;
            s.tools.stamp.mode = SurfaceMode::Project;
            s.tools.stamp.size_frac = 0.4;
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the image stamp on the piece", |s, sc| flow_click_on_piece(s, sc, 0.5)),
        Step::WaitIdle,
        Step::Assert("the projected stamp inked the piece, not the hidden board", |s, sc| flow_stamp_check(s, sc, false, "projected stamp")),
        // (8) A wrapped stamp low on the piece with "jump small gaps" on (the board touches the piece's foot).
        Step::Setup("Wrap with jump small gaps", |s, _| {
            s.tools.stamp.mode = SurfaceMode::Wrap;
            s.tools.stamp.wrap = WrapOptions { jump_gaps: true, overlap: false };
            s.tools.stamp.size_frac = 0.4;
            Ok(())
        }),
        // A placement switches to the Transform tool (whose click would select the stamp under it): the Stamp tool again.
        Step::Do(AppAction::SelectStamp(None)),
        Step::Do(AppAction::SetTool(ToolKind::Stamp)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the wrapped stamp low on the piece", |s, sc| flow_click_on_piece(s, sc, 0.25)),
        Step::WaitIdle,
        Step::Assert("a new wrapped stamp inked the piece, not the hidden board", |s, sc| {
            let (id, placement) = placed_stamp(s, false)?;
            match placement {
                Placement::Projected(p) if p.mode == SurfaceMode::Wrap && p.wrap.jump_gaps => {}
                other => return Err(format!("the selected stamp {id:?} is not a wrapped one with jump gaps: {other:?}")),
            }
            if doc_of(s)?.stamps.len() != 2 {
                return Err(format!("{} stamps, expected the projected and the wrapped one", doc_of(s)?.stamps.len()));
            }
            flow_stamp_check(s, sc, false, "wrapped stamp")
        }),
        // (9) A text on the piece.
        Step::Setup("text content", |s, _| {
            s.tools.stamp.mode = SurfaceMode::Project;
            s.tools.stamp.wrap = WrapOptions::default();
            s.tools.text.content = "WTE".to_owned();
            s.tools.text.em_frac = 0.2;
            s.tools.text.color = [20, 20, 20, 255];
            Ok(())
        }),
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Do(AppAction::SelectStamp(None)),
        Step::WaitIdle,
        Step::WaitFrames(2),
        Step::Dyn("place the text on the piece", |s, sc| flow_click_on_piece(s, sc, 0.6)),
        Step::WaitIdle,
        Step::Assert("the text inked the piece, not the hidden board", |s, sc| flow_stamp_check(s, sc, true, "text")),
        Step::Key(egui::Modifiers::NONE, egui::Key::Escape),
        // (9b) Changing visibility never re-bakes a stamp: the wrapped stamp selected with the Transform tool (its
        // parameterization is dropped and made again for the new visibility), Show all, the board hidden again.
        Step::Dyn("select the wrapped stamp with the Transform tool", |s, _| {
            let doc = doc_of(s)?;
            let id = doc
                .stamps
                .iter()
                .find(|st| matches!(st.placement, Placement::Projected(p) if p.mode == SurfaceMode::Wrap))
                .map(|st| st.id)
                .ok_or("there is no wrapped stamp")?;
            Ok(vec![Step::Do(AppAction::SetTool(ToolKind::Transform)), Step::Do(AppAction::SelectStamp(Some(id)))])
        }),
        Step::WaitIdle,
        Step::WaitFrames(5),
        Step::Setup("remember the composites and the history", |s, sc| {
            sc.values.insert("flow_before_toggle", composite_hash(doc_of(s)?));
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::ShowAllObjects),
        Step::WaitIdle,
        Step::WaitFrames(10),
        Step::Assert("Show all re-baked nothing", |s, sc| {
            hidden_are(s, &[])?;
            if composite_hash(doc_of(s)?) != remembered(sc, "flow_before_toggle")? {
                return Err("showing the board changed the composites (a stamp was baked again)".to_owned());
            }
            undo_since(s, sc, 0)
        }),
        Step::Dyn("hide the board again", |_, sc| {
            Ok(vec![Step::Do(AppAction::SelectObject(remembered(sc, "big")? as u32)), Step::Do(AppAction::HideSelectedObjects)])
        }),
        Step::WaitIdle,
        Step::WaitFrames(10),
        Step::Assert("hiding the board again re-baked nothing", |s, sc| {
            hidden_are(s, &[remembered(sc, "big")? as u32])?;
            if composite_hash(doc_of(s)?) != remembered(sc, "flow_before_toggle")? {
                return Err("hiding the board changed the composites (a stamp was baked again)".to_owned());
            }
            board_untouched(s, sc)?;
            undo_since(s, sc, 0)
        }),
        Step::Do(AppAction::SelectStamp(None)),
        Step::Do(AppAction::SetTool(ToolKind::Navigate)),
        Step::Dyn("select the piece again", |_, sc| Ok(vec![Step::Do(AppAction::SelectObject(remembered(sc, "piece")? as u32))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the piece is selected and tinted (Navigate), the board hidden", |s, sc| {
            selection_is(s, &[remembered(sc, "piece")? as u32])?;
            hidden_are(s, &[remembered(sc, "big")? as u32])?;
            if crate::view::view3d::highlight_shown(s) { Ok(()) } else { Err("the selection tint is not shown with Navigate".to_owned()) }
        }),
        Step::Shot { name: "designed", expect: "" },
        // (10) Preview with the selection (no tint there, the board left out).
        Step::Do(AppAction::SetViewMode(ViewMode::Preview)),
        Step::WaitIdle,
        Step::WaitFrames(30),
        Step::Shot { name: "preview", expect: "" },
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::WaitFrames(3),
        // (11) Save, reopen: the board hidden again (flags and rays), the composites as saved, nothing selected.
        Step::Setup("remember the saved composites", |s, sc| {
            let doc = doc_of(s)?;
            sc.values.insert("flow_saved", composite_hash(doc));
            sc.values.insert("flow_structure", structure_key(doc));
            Ok(())
        }),
        Step::Dyn("Save As the project", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::Assert("the project was saved", |s, sc| {
            if session_of(s)?.doc.is_modified() {
                return Err("the document is still modified after the save".to_owned());
            }
            file_non_empty(&with_suffix(&sc.out_stem, ".wtep"))
        }),
        Step::Dyn("open the saved project", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(with_suffix(&sc.out_stem, ".wtep"))))])),
        Step::WaitIdle,
        Step::WaitFrames(3),
        Step::Assert("the reopened project hides the board (flags and rays), says so, equals the saved one", |s, sc| {
            let big = remembered(sc, "big")? as u32;
            let session = session_of(s)?;
            if session.path.as_deref() != Some(with_suffix(&sc.out_stem, ".wtep").as_path()) {
                return Err(format!("the open document is {:?}", session.path));
            }
            hidden_are(s, &[big])?;
            selection_is(s, &[])?;
            if session.doc.is_modified() {
                return Err("the reopened document is modified".to_owned());
            }
            let (on_big, total) = board_ray_hits(&session.doc, big);
            if on_big != 0 {
                return Err(format!("after the reopen {on_big} of {total} rays down hit the hidden board"));
            }
            let name = &session.doc.mesh.objects[big as usize].name;
            let texts = s.toasts.visible_labels();
            let hidden: Vec<&String> = texts.iter().filter(|t| t.contains("hidden")).collect();
            if hidden.len() != 1 || !hidden[0].contains(name.as_str()) {
                return Err(format!("the toasts about hidden objects are {hidden:?} (all: {texts:?})"));
            }
            if structure_key(&session.doc) != remembered(sc, "flow_structure")? {
                return Err("the sets, layers or stamps differ after the round trip".to_owned());
            }
            if composite_hash(&session.doc) != remembered(sc, "flow_saved")? {
                return Err("the composites differ after the round trip".to_owned());
            }
            board_untouched(s, sc)
        }),
        Step::Shot { name: "reopened", expect: "" },
        // (12) Export one layer (the piece's paint layer), then every set's flattened PNG: hiding changes no export.
        Step::Dyn("export the piece's paint layer", |s, sc| {
            let dir = fresh_out_dir(sc, "_export_layer")?;
            let layer = flow_paint_layer(s, sc)?;
            let opts = ExportOptions { dir, flat: None, per_layer_png: true, ora: false, psd: false, layers: Some(vec![layer]), ..export_options(s, sc)? };
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("exactly the paint layer's PNG, equal to the layer, with the stroke", |s, sc| {
            assert_no_dialog(s, sc)?;
            let layer = flow_paint_layer(s, sc)?;
            let images = expect_layer_files(s, &with_suffix(&sc.out_stem, "_export_layer"), &[layer])?;
            let n = ink(&images[0]);
            log::info!("objects_flow_path: the exported paint layer has {n} inked texels");
            if n == 0 { Err("the exported paint layer is empty".to_owned()) } else { Ok(()) }
        }),
        Step::Dyn("export every set's flattened PNG", |s, sc| {
            let dir = fresh_out_dir(sc, "_export_flat")?;
            let opts = ExportOptions { dir, per_layer_png: false, ora: false, psd: false, ..export_options(s, sc)? };
            Ok(vec![Step::Do(AppAction::RunExport(opts))])
        }),
        Step::WaitIdle,
        Step::Assert("every set's flattened PNG (the hidden board's too) holds its composite", |s, sc| {
            assert_no_dialog(s, sc)?;
            let doc = doc_of(s)?;
            let opts = ExportOptions {
                dir: with_suffix(&sc.out_stem, "_export_flat"),
                per_layer_png: false,
                ora: false,
                psd: false,
                ..export_options(s, sc)?
            };
            let files = read_export_dir(doc, &opts)?;
            if files.len() != doc.sets.len() {
                return Err(format!("{} files for {} sets", files.len(), doc.sets.len()));
            }
            let summary = verify_export(doc, &opts, &files)?;
            log::info!("objects_flow_path: flattened export: {summary}");
            Ok(())
        }),
        // (13) Show all: the rays hit the board again.
        Step::Do(AppAction::ShowAllObjects),
        Step::WaitFrames(3),
        Step::Assert("Show all: nothing hidden, the rays hit the board again", |s, sc| {
            hidden_are(s, &[])?;
            let (on_big, _) = board_ray_hits(doc_of(s)?, remembered(sc, "big")? as u32);
            if on_big == 0 { Err("with everything shown no ray down hits the board".to_owned()) } else { Ok(()) }
        }),
        // (14) The other way round: the piece hidden, a short stroke on its screen centre paints what is behind it and
        // leaves the piece's sets alone (a hidden object neither blocks a stroke nor takes its dabs).
        Step::Do(AppAction::FrameModel),
        Step::WaitFrames(3),
        Step::Dyn("select and hide the piece", |_, sc| {
            Ok(vec![Step::Do(AppAction::SelectObject(remembered(sc, "piece")? as u32)), Step::Do(AppAction::HideSelectedObjects)])
        }),
        Step::WaitFrames(3),
        Step::Setup("the ray through the hidden piece reaches an object with other texture sets", |s, sc| {
            let piece = remembered(sc, "piece")? as u32;
            hidden_are(s, &[piece])?;
            let xf = view_xf(s, ViewId::Main3D)?;
            let doc = doc_of(s)?;
            let c = world_screen(&xf, doc.mesh.objects[piece as usize].bounds.center())?;
            let ray = xf.camera.ray(c, xf.rect, xf.scene_radius);
            let shown = doc.bvh.with_hidden(None).raycast(&doc.mesh, &ray, 1e-5 * doc.mesh.diag(), f32::INFINITY, wte_core::Cull::None);
            if shown.map(|h| doc.mesh.tri_object_of(h.tri)) != Some(piece) {
                return Err("with the piece shown the ray through its centre does not hit it".to_owned());
            }
            let Surface::Model(hit) = xf.surface_at(doc, TextureSetId(0), c) else {
                return Err("the ray through the hidden piece hits nothing".to_owned());
            };
            let behind = doc.mesh.tri_object_of(hit.tri);
            let sets = object_sets(doc);
            if behind == piece || !sets[behind as usize].is_disjoint(&sets[piece as usize]) {
                return Err(format!("the ray through the hidden piece hits object {behind}, which shares its texture sets"));
            }
            log::info!("objects_flow_path: behind the hidden piece: {:?}", doc.mesh.objects[behind as usize].name);
            let (piece_sets, behind_sets): (Vec<u32>, Vec<u32>) =
                (sets[piece as usize].iter().copied().collect(), sets[behind as usize].iter().copied().collect());
            sc.values.insert("behind", u64::from(behind));
            sc.values.insert("piece_sets_2", sets_hash(doc, &piece_sets)?);
            sc.values.insert("behind_sets", sets_hash(doc, &behind_sets)?);
            sc.values.insert("flow_c2", pack(Vec2::new(c.x, c.y)));
            remember_undo(s, sc)
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::Setup("a small brush (the neighbouring pieces share the piece's texture sets)", |s, sc| {
            s.tools.brush.size_px = 6.0;
            log::info!("objects_flow_path: behind the hidden piece: object {}", remembered(sc, "behind")?);
            Ok(())
        }),
        Step::Dyn("a short stroke on the hidden piece's centre", |s, sc| {
            let xf = view_xf(s, ViewId::Main3D)?;
            let c = unpack(remembered(sc, "flow_c2")?);
            Ok(vec![drag(&xf, &[egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x + 4.0, c.y)])])
        }),
        Step::WaitIdle,
        Step::Assert("the stroke painted what is behind the hidden piece, not the piece; a Brush Stroke entry", |s, sc| {
            let doc = doc_of(s)?;
            let sets = object_sets(doc);
            let set_list = |o: u64| sets[o as usize].iter().copied().collect::<Vec<u32>>();
            if sets_hash(doc, &set_list(remembered(sc, "piece")?))? != remembered(sc, "piece_sets_2")? {
                return Err("the stroke changed the hidden piece's texture sets".to_owned());
            }
            if sets_hash(doc, &set_list(remembered(sc, "behind")?))? == remembered(sc, "behind_sets")? {
                return Err("the stroke did not paint what is behind the hidden piece".to_owned());
            }
            // The stroke ends with a "Brush Stroke" entry; a set whose active layer is a stamp or text layer (the
            // white pawns' sets after the stamps) first gets an empty paint layer of its own (step 4 of the Brush row).
            if doc.has_pending_edit() || doc.undo_label() != Some("Brush Stroke") || (doc.history.undo_len() as u64) <= remembered(sc, "undo")? {
                return Err(format!("the history ends with {:?} ({} entries)", doc.undo_label(), doc.history.undo_len()));
            }
            Ok(())
        }),
        Step::Do(AppAction::ShowAllObjects),
        Step::WaitFrames(3),
    ]);
    steps
}

/// The board (largest ground footprint) and the piece (the first other object named "pawn", else the other object with
/// the most triangles).
fn flow_choose(doc: &Document) -> Result<(u32, u32), String> {
    let objects = &doc.mesh.objects;
    if objects.len() < 2 {
        return Err(format!("the model has {} object(s); objects_flow_path needs several", objects.len()));
    }
    let foot = |o: u32| {
        let b = objects[o as usize].bounds;
        (b.max.x - b.min.x).max(0.0) * (b.max.z - b.min.z).max(0.0)
    };
    let n = objects.len() as u32;
    let big = (0..n).max_by(|&a, &b| foot(a).total_cmp(&foot(b))).ok_or("no object")?;
    let piece = (0..n)
        .filter(|&o| o != big)
        .find(|&o| objects[o as usize].name.to_lowercase().contains("pawn"))
        .or_else(|| (0..n).filter(|&o| o != big).max_by_key(|&o| objects[o as usize].tri_count))
        .ok_or("no piece")?;
    Ok((big, piece))
}

/// The texture sets each object's triangles use.
fn object_sets(doc: &Document) -> Vec<std::collections::BTreeSet<u32>> {
    let mut out = vec![std::collections::BTreeSet::new(); doc.mesh.objects.len()];
    for t in 0..doc.mesh.tri_count() {
        if let (Some(set), Some(sets)) = (doc.mesh.tri_set(t), out.get_mut(doc.mesh.tri_object_of(t) as usize)) {
            sets.insert(set.0);
        }
    }
    out
}

/// The sets object `big` uses and no other object does.
fn board_only_sets(sets: &[std::collections::BTreeSet<u32>], big: u32) -> Vec<u32> {
    sets[big as usize].iter().copied().filter(|s| sets.iter().enumerate().all(|(o, os)| o == big as usize || !os.contains(s))).collect()
}

/// One hash of the composites of `sets`.
fn sets_hash(doc: &Document, sets: &[u32]) -> Result<u64, String> {
    sets.iter().try_fold(0xcbf2_9ce4_8422_2325u64, |h, &set| Ok(wte_core::fnv1a64(h, &set_hash(doc, set as usize)?.to_le_bytes())))
}

/// The sets only the board uses still hold their remembered composites.
fn board_untouched(s: &AppState, sc: &Scratch) -> Result<(), String> {
    let doc = doc_of(s)?;
    let board = board_only_sets(&object_sets(doc), remembered(sc, "big")? as u32);
    if sets_hash(doc, &board)? != remembered(sc, "board_sets")? {
        return Err(format!("the texture sets only the hidden board uses ({board:?}) changed"));
    }
    Ok(())
}

/// Rays straight down over a 12 x 12 grid of the board's footprint (the document's masked raycast): hits on the
/// board, all hits.
fn board_ray_hits(doc: &Document, big: u32) -> (usize, usize) {
    let b = doc.mesh.objects[big as usize].bounds;
    let top = b.max.y + doc.mesh.diag();
    let (mut on_big, mut total) = (0, 0);
    for i in 0..12 {
        for j in 0..12 {
            let x = b.min.x + (b.max.x - b.min.x) * (0.05 + 0.9 * (i as f32 + 0.5) / 12.0);
            let z = b.min.z + (b.max.z - b.min.z) * (0.05 + 0.9 * (j as f32 + 0.5) / 12.0);
            if let Some(hit) = doc.raycast(&wte_core::Ray::new(Vec3::new(x, top, z), Vec3::NEG_Y)) {
                total += 1;
                if doc.mesh.tri_object_of(hit.tri) == big {
                    on_big += 1;
                }
            }
        }
    }
    (on_big, total)
}

/// The stroke of `objects_flow_path`: from the piece's projected centre (whose masked ray hits the piece) to the first
/// point around it, 1.2 to 2.5 piece radii away, whose ray hits the board when the board is shown and nothing now.
/// Fails when there is none (the stroke would not test the hidden board).
fn flow_stroke_points(s: &AppState, sc: &Scratch) -> Result<(ViewXform, egui::Pos2, egui::Pos2), String> {
    let (big, piece) = (remembered(sc, "big")? as u32, remembered(sc, "piece")? as u32);
    let xf = view_xf(s, ViewId::Main3D)?;
    let doc = doc_of(s)?;
    let bounds = doc.mesh.objects[piece as usize].bounds;
    let c = world_screen(&xf, bounds.center())?;
    match xf.surface_at(doc, TextureSetId(0), c) {
        Surface::Model(hit) if doc.mesh.tri_object_of(hit.tri) == piece => {}
        _ => return Err("the ray through the piece's centre does not hit the piece".to_owned()),
    }
    let half = (bounds.max.x - bounds.min.x).max(bounds.max.z - bounds.min.z) * 0.5;
    let r = xf.world_len_on_screen(bounds.center(), half).ok_or("the piece is behind the camera")?;
    let shown = doc.bvh.with_hidden(None);
    let t_min = 1e-5 * doc.mesh.diag();
    for k in [1.2f32, 1.6, 2.0, 2.5] {
        for a in 0..16 {
            // Downwards first (in front of the piece's foot), then round.
            let angle = std::f32::consts::FRAC_PI_2 + a as f32 * std::f32::consts::TAU / 16.0;
            let e = c + egui::vec2(angle.cos(), angle.sin()) * r * k;
            if !xf.rect.shrink(8.0).contains(e) {
                continue;
            }
            let ray = xf.camera.ray(e, xf.rect, xf.scene_radius);
            let on_board =
                shown.raycast(&doc.mesh, &ray, t_min, f32::INFINITY, wte_core::Cull::None).is_some_and(|h| doc.mesh.tri_object_of(h.tri) == big);
            if on_board && matches!(xf.surface_at(doc, TextureSetId(0), e), Surface::Miss) {
                return Ok((xf, c, e));
            }
        }
    }
    Err("no point around the piece shows the board (when shown) and nothing now".to_owned())
}

/// A click on the piece's vertical axis at `height` (0 = foot, 1 = top) of its bounds; fails when that ray does not hit
/// the piece.
fn flow_click_on_piece(s: &AppState, sc: &Scratch, height: f32) -> Result<Vec<Step>, String> {
    let piece = remembered(sc, "piece")? as u32;
    let xf = view_xf(s, ViewId::Main3D)?;
    let doc = doc_of(s)?;
    let b = doc.mesh.objects[piece as usize].bounds;
    let c = b.center();
    let p = world_screen(&xf, Vec3::new(c.x, b.min.y + (b.max.y - b.min.y) * height, c.z))?;
    match xf.surface_at(doc, TextureSetId(0), p) {
        Surface::Model(hit) if doc.mesh.tri_object_of(hit.tri) == piece => Ok(vec![click(ViewId::Main3D, norm(&xf, p))]),
        _ => Err(format!("the ray at {height} of the piece's height does not hit the piece")),
    }
}

/// The selected stamp (`text` or image) inked the piece's sets and no layer of a set only the board uses; the board's
/// sets are unchanged.
fn flow_stamp_check(s: &AppState, sc: &Scratch, text: bool, what: &str) -> Result<(), String> {
    let (id, _) = placed_stamp(s, text)?;
    let doc = doc_of(s)?;
    let sets = object_sets(doc);
    let piece = remembered(sc, "piece")? as usize;
    let board = board_only_sets(&sets, remembered(sc, "big")? as u32);
    let mut on_piece = 0;
    for (set, layer) in doc.stamp_layers(id) {
        let (Some(st), Some(l)) = (doc.set(set), doc.layer(layer)) else { continue };
        let n = ink(&layer_canvas(l, st.width, st.height));
        if board.contains(&set.0) && n > 0 {
            return Err(format!("the {what} put {n} texels into the hidden board's set {}", set.0));
        }
        if sets[piece].contains(&set.0) {
            on_piece += n;
        }
    }
    log::info!("objects_flow_path: {what}: {on_piece} texels in the piece's sets, layers {:?}", doc.stamp_layers(id));
    if on_piece == 0 {
        return Err(format!("the {what} put nothing into the piece's sets"));
    }
    board_untouched(s, sc)
}

/// The paint layer the stroke went into: the first non-Base raster layer with ink in a set of the piece.
fn flow_paint_layer(s: &AppState, sc: &Scratch) -> Result<LayerId, String> {
    let doc = doc_of(s)?;
    let piece = remembered(sc, "piece")? as usize;
    for &set in &object_sets(doc)[piece] {
        let st = doc.set(TextureSetId(set)).ok_or("no such set")?;
        for l in st.layers.iter().skip(1) {
            if let LayerKind::Raster(pixels) = &l.kind
                && ink(pixels) > 0
            {
                return Ok(l.id);
            }
        }
    }
    Err("no painted raster layer in the piece's sets".to_owned())
}
