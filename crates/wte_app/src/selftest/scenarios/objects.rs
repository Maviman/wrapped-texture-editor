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
        Step::Shot("selected"),
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
        Step::Shot("hidden"),
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitFrames(3),
        Step::Shot("hidden_2d"),
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
        Step::Shot("shown"),
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
        Step::Shot("isolated"),
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
        Step::Shot("isolated"),
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
