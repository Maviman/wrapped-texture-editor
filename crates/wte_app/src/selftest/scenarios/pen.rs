// Branch `pen-input` (PEN_INPUT_PLAN 4.5, ARCHITECTURE.md 8.11): the scenario `pen` — pen input fed in-process as
// recorded pen samples (`Step::Pen`: the events egui-winit makes for a Windows Ink pen plus the matching `PenRecord`s of
// the window procedure), never through the OS. On the Sphere sample: a 2D pressure ramp (width follows pressure), mouse
// vs pen at full pressure (identical pixels, 2D and 3D), flow dynamics, a light and a full 3D stroke, the eraser end
// (the tool switches to the Eraser and back), the barrel button (orbits, paints nothing), a pen tap on a Layers row (a
// click), the status readout during contact and Help > Tablet setup. Shots: `pen_ramp` (the 2D ramp), `pen_3d` (light
// and full 3D strokes), `tablet_help`; the final screenshot shows a 3D ramp.

use super::*;
use crate::selftest::PenPoint;

/// Brush diameter of the scenario (texels).
pub(super) const SIZE: f32 = 40.0;
/// The 2D ramp's row and ends (UV).
pub(super) const RAMP_V: f32 = 0.5;
pub(super) const RAMP_U: [f32; 2] = [0.15, 0.85];

/// The `pen` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn pen() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        // (1) Size 40, hard, full flow; pressure drives the size from 0, not the flow.
        Step::Setup("brush: size 40, hardness 1, flow 1, pressure Size on (min 0), Flow off", |s, _| {
            s.tools.brush = BrushSettings {
                size_px: SIZE,
                hardness: 1.0,
                flow: 1.0,
                pressure_size: true,
                pressure_size_min: 0.0,
                pressure_flow: false,
                ..BrushSettings::default()
            };
            Ok(())
        }),
        Step::WaitIdle,
        Step::Assert("no pen yet: the readout is empty", |s, _| {
            if s.pen.frame.readout().is_none() { Ok(()) } else { Err(format!("readout {:?}", s.pen.frame.readout())) }
        }),
        Step::Setup("reset the pen probes", reset_probes),
        // (2) The 2D ramp, pressure 0.1 -> 1.0.
        Step::Dyn("2D pen ramp, pressure 0.1 -> 1.0", |s, _| uv_pen(s, RAMP_V, RAMP_U, (0.1, 1.0), false, false)),
        Step::WaitIdle,
        Step::Assert("the ramp is thin at 20 % and 40 texels at the end", assert_ramp),
        Step::Assert("the readout showed \"Pen N %\" while the pen touched", |_, sc| {
            let frames = sc.values.get("pen_contact_frames").copied().unwrap_or(0);
            let ok = sc.values.get("pen_readout_ok").copied().unwrap_or(0);
            if frames > 0 && ok == 1 { Ok(()) } else { Err(format!("{frames} contact frames, readout ok {ok}")) }
        }),
        Step::Assert("after the pen left: no readout, the pen was seen", |s, _| {
            if s.pen.frame.readout().is_none() && s.pen.frame.seen && !s.pen.frame.in_contact {
                Ok(())
            } else {
                Err(format!("{:?}", s.pen.frame))
            }
        }),
        Step::Shot { name: "pen_ramp", expect: "" },
        // (3) 2D identity: the mouse and the pen at full pressure paint the same pixels.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Dyn("2D mouse stroke", |s, _| uv_mouse(s, 0.35)),
        Step::WaitIdle,
        Step::Setup("remember the mouse stroke", |s, sc| remember_paint(s, sc, "mouse_2d")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Dyn("2D pen stroke at full pressure", |s, _| uv_pen(s, 0.35, RAMP_U, (1.0, 1.0), false, false)),
        Step::WaitIdle,
        Step::Assert("2D: pen at full pressure == mouse", |s, sc| same_paint(s, sc, "mouse_2d")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        // (4) Flow dynamics: Size off, Flow on from 0, brush flow 0.5.
        Step::Setup("brush: Size off, Flow on (min 0), flow 0.5", |s, _| {
            let b = &mut s.tools.brush;
            (b.pressure_size, b.pressure_flow, b.pressure_flow_min, b.flow) = (false, true, 0.0, 0.5);
            Ok(())
        }),
        Step::Dyn("2D pen at constant pressure 0.2", |s, _| uv_pen(s, 0.3, RAMP_U, (0.2, 0.2), false, false)),
        Step::WaitIdle,
        Step::Dyn("2D pen at constant pressure 1.0", |s, _| uv_pen(s, 0.7, RAMP_U, (1.0, 1.0), false, false)),
        Step::WaitIdle,
        Step::Assert("flow follows pressure: the light row is fainter, not empty", |s, _| {
            let light = alpha_at(s, [0.5, 0.3])?;
            let full = alpha_at(s, [0.5, 0.7])?;
            if light > 0 && light < full { Ok(()) } else { Err(format!("centre alpha {light} (0.2) vs {full} (1.0)")) }
        }),
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Setup("brush: Size on (min 0), Flow off, flow 1", |s, _| {
            let b = &mut s.tools.brush;
            (b.pressure_size, b.pressure_size_min, b.pressure_flow, b.flow) = (true, 0.0, false, 1.0);
            Ok(())
        }),
        // (5) 3D: a light stroke covers far fewer texels than a full one; full pressure == mouse.
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Dyn("3D pen stroke at pressure 0.25", |s, _| model_pen(s, &[[-0.5, -0.1], [0.5, 0.1]], (0.25, 0.25), false, false)),
        Step::WaitIdle,
        Step::Setup("count the light stroke", |s, sc| count_paint(s, sc, "light_3d")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Dyn("3D pen stroke at pressure 1.0", |s, _| model_pen(s, &[[-0.5, -0.1], [0.5, 0.1]], (1.0, 1.0), false, false)),
        Step::WaitIdle,
        Step::Setup("count the full stroke", |s, sc| count_paint(s, sc, "full_3d")),
        Step::Assert("3D: 0 < light < half of full", |_, sc| {
            let (light, full) = (remembered(sc, "light_3d")?, remembered(sc, "full_3d")?);
            if light > 0 && light * 2 < full { Ok(()) } else { Err(format!("{light} texels at 0.25, {full} at 1.0")) }
        }),
        Step::Setup("remember the pen stroke", |s, sc| remember_paint(s, sc, "pen_3d")),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Dyn("3D mouse stroke", |s, _| model_stroke(s, ViewId::Main3D, &[[-0.5, -0.1], [0.5, 0.1]])),
        Step::WaitIdle,
        Step::Assert("3D: pen at full pressure == mouse", |s, sc| same_paint(s, sc, "pen_3d")),
        Step::Dyn("a light 3D stroke beside it, for the shot", |s, _| {
            model_pen(s, &[[-0.5, 0.35], [0.5, 0.45]], (0.25, 0.25), false, false)
        }),
        Step::WaitIdle,
        Step::Shot { name: "pen_3d", expect: "" },
        Step::Do(AppAction::Undo),
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
    ];
    // (6) The eraser end: a patch in 2D, then the inverted pen across it.
    steps.extend([
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitIdle,
        Step::Dyn("a patch: a wide mouse stroke", |s, _| uv_mouse(s, RAMP_V)),
        Step::WaitIdle,
        Step::Assert("the patch is painted", |s, _| {
            let a = alpha_at(s, [0.5, RAMP_V])?;
            if a == 255 { Ok(()) } else { Err(format!("alpha {a}")) }
        }),
        Step::Setup("reset the pen probes", reset_probes),
        Step::Dyn("the eraser end: two hover samples, then a stroke across the patch", |s, _| {
            let xf = view_xf(s, ViewId::Main2D)?;
            let a = norm(&xf, uv_screen(&xf, [0.5, RAMP_V - 0.12]));
            let b = norm(&xf, uv_screen(&xf, [0.5, RAMP_V + 0.12]));
            let mut samples = vec![PenPoint { at: [a[0] - 0.02, a[1]], pressure: 0.0 }, PenPoint { at: a, pressure: 0.0 }];
            samples.extend((0..=12).map(|k| {
                let t = k as f32 / 12.0;
                PenPoint { at: [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], pressure: 1.0 }
            }));
            Ok(vec![Step::Pen { view: ViewId::Main2D.into(), samples, inverted: true, barrel: false, points_per_frame: 2 }])
        }),
        Step::Assert("the stroke erased the centre and the tool was the Eraser while touching", |s, sc| {
            let a = alpha_at(s, [0.5, RAMP_V])?;
            let (contact, eraser) =
                (sc.values.get("pen_contact_frames").copied().unwrap_or(0), sc.values.get("pen_eraser_frames").copied().unwrap_or(0));
            if a != 0 {
                return Err(format!("alpha {a} in the erased centre"));
            }
            if contact == 0 || eraser != contact {
                return Err(format!("the Eraser in {eraser} of {contact} contact frames"));
            }
            if s.doc().and_then(|d| d.undo_label()) != Some("Erase") {
                return Err(format!("the last entry is {:?}", s.doc().and_then(|d| d.undo_label())));
            }
            Ok(())
        }),
        Step::WaitFrames(1),
        Step::Assert("the pen left: the Brush is back", |s, _| {
            if s.tool == ToolKind::Brush && s.pen.eraser_restore.is_none() { Ok(()) } else { Err(format!("tool {:?}", s.tool)) }
        }),
        // (7) The barrel button: orbits in 3D, paints nothing.
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Setup("remember the camera and the paint", |s, sc| {
            let cam = s.session.as_ref().ok_or("no session")?.camera;
            sc.values.insert("yaw", u64::from(cam.yaw.to_bits()));
            sc.values.insert("pitch", u64::from(cam.pitch.to_bits()));
            remember_paint(s, sc, "before_barrel")
        }),
        Step::Dyn("a barrel drag over the model", |s, _| model_pen(s, &[[-0.4, 0.0], [0.4, 0.2]], (0.8, 0.8), false, true)),
        Step::WaitIdle,
        Step::Assert("the camera turned and nothing was painted", |s, sc| {
            let cam = s.session.as_ref().ok_or("no session")?.camera;
            let turned = u64::from(cam.yaw.to_bits()) != remembered(sc, "yaw")?
                || u64::from(cam.pitch.to_bits()) != remembered(sc, "pitch")?;
            if !turned {
                return Err("the camera did not move".to_owned());
            }
            same_paint(s, sc, "before_barrel")
        }),
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        // (8) A pen tap on the Base row selects it.
        Step::Dyn("a pen tap on the name of Base", |s, _| {
            let base = doc_of(s)?.sets[0].layers[0].id;
            let at = panel_rect(s, PanelHit::LayerName(base))?.center();
            let xf = view_xf(s, ViewId::Main3D)?;
            let samples = vec![PenPoint { at: norm(&xf, at), pressure: 0.0 }, PenPoint { at: norm(&xf, at), pressure: 0.6 }];
            Ok(vec![Step::Pen { view: ViewId::Main3D.into(), samples, inverted: false, barrel: false, points_per_frame: 1 }])
        }),
        Step::WaitIdle,
        Step::Assert("the tap selected Base", |s, _| {
            let set = &doc_of(s)?.sets[0];
            if set.active == Some(set.layers[0].id) { Ok(()) } else { Err(format!("the active layer is {:?}", set.active)) }
        }),
        Step::Dyn("select Paint 1 again", |s, _| row_click(s, 1, PanelHit::LayerName)),
        Step::WaitIdle,
        // (9) The final picture: a 3D ramp.
        Step::Dyn("3D pen ramp for the final screenshot", |s, _| {
            model_pen(s, &[[-0.6, -0.2], [0.0, 0.05], [0.6, 0.2]], (0.05, 1.0), false, false)
        }),
        Step::WaitIdle,
        Step::Assert("the ramp painted", |s, _| {
            if paint_count(s)? > 0 { Ok(()) } else { Err("nothing painted".to_owned()) }
        }),
        // (10) Help > Tablet setup opens and closes.
        Step::Do(AppAction::ShowDialog(Dialog::TabletHelp)),
        Step::WaitIdle,
        Step::Assert("Tablet setup is open", |s, _| {
            if matches!(s.dialog, Dialog::TabletHelp) { Ok(()) } else { Err(format!("dialog {}", dialog_name(&s.dialog))) }
        }),
        Step::Shot { name: "tablet_help", expect: "" },
        Step::Do(AppAction::CloseDialog),
        Step::WaitIdle,
        Step::Assert("Tablet setup closed", |s, _| {
            if matches!(s.dialog, Dialog::None) { Ok(()) } else { Err(format!("dialog {}", dialog_name(&s.dialog))) }
        }),
    ]);
    steps
}

/// Clears the probes the runner writes while a pen touches (`Runner::probe_pen`).
pub(super) fn reset_probes(_: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    for key in ["pen_contact_frames", "pen_eraser_frames", "pen_readout_ok"] {
        sc.values.insert(key, 0);
    }
    Ok(())
}

/// Pen samples along screen points: one hover sample at the start, then contact samples with the pressure ramping
/// linearly along the path from `p.0` to `p.1`.
fn pen_samples(xf: &ViewXform, screen: &[egui::Pos2], p: (f32, f32)) -> Vec<PenPoint> {
    let points = densify(screen);
    let n = points.len().max(2) - 1;
    let mut samples = vec![PenPoint { at: norm(xf, points[0]), pressure: 0.0 }];
    samples.extend(points.iter().enumerate().map(|(k, q)| PenPoint {
        at: norm(xf, *q),
        pressure: p.0 + (p.1 - p.0) * (k as f32 / n as f32),
    }));
    samples
}

/// A pen stroke in the 2D view along the row `v` from `u[0]` to `u[1]`.
fn uv_pen(state: &AppState, v: f32, u: [f32; 2], p: (f32, f32), inverted: bool, barrel: bool) -> Result<Vec<Step>, String> {
    let xf = view_xf(state, ViewId::Main2D)?;
    let screen = [uv_screen(&xf, [u[0], v]), uv_screen(&xf, [u[1], v])];
    let samples = pen_samples(&xf, &screen, p);
    Ok(vec![Step::Pen { view: ViewId::Main2D.into(), samples, inverted, barrel, points_per_frame: POINTS_PER_FRAME }])
}

/// The mouse stroke of `uv_pen` (same points, same points per frame).
fn uv_mouse(state: &AppState, v: f32) -> Result<Vec<Step>, String> {
    uv_stroke(state, &[[RAMP_U[0], v], [RAMP_U[1], v]])
}

/// A pen stroke over the model along disc coordinates (the points of `model_stroke`).
fn model_pen(state: &AppState, disc: &[[f32; 2]], p: (f32, f32), inverted: bool, barrel: bool) -> Result<Vec<Step>, String> {
    let (xf, c, r) = model_disc(state, ViewId::Main3D)?;
    let screen: Vec<egui::Pos2> = disc.iter().map(|q| disc_point(c, r, *q)).collect();
    let samples = pen_samples(&xf, &screen, p);
    Ok(vec![Step::Pen { view: ViewId::Main3D.into(), samples, inverted, barrel, points_per_frame: POINTS_PER_FRAME }])
}

/// FNV hash of every raster layer above Base of every set (the painted pixels).
pub(super) fn paint_hash(state: &AppState) -> Result<u64, String> {
    let doc = doc_of(state)?;
    Ok(doc.sets.iter().flat_map(|s| s.layers.iter().skip(1)).fold(0xcbf2_9ce4_8422_2325, |h, layer| match &layer.kind {
        LayerKind::Raster(buf) => wte_core::fnv1a64(h, &buf.data),
        _ => h,
    }))
}

/// Texels with alpha > 0 in every raster layer above Base.
pub(super) fn paint_count(state: &AppState) -> Result<u64, String> {
    let doc = doc_of(state)?;
    Ok(doc
        .sets
        .iter()
        .flat_map(|s| s.layers.iter().skip(1))
        .map(|l| match &l.kind {
            LayerKind::Raster(buf) => buf.data.chunks(4).filter(|p| p[3] > 0).count() as u64,
            _ => 0,
        })
        .sum())
}

pub(super) fn remember_paint(state: &mut AppState, sc: &mut Scratch, key: &'static str) -> Result<(), String> {
    let h = paint_hash(state)?;
    sc.values.insert(key, h);
    Ok(())
}

pub(super) fn count_paint(state: &mut AppState, sc: &mut Scratch, key: &'static str) -> Result<(), String> {
    let n = paint_count(state)?;
    sc.values.insert(key, n);
    Ok(())
}

pub(super) fn same_paint(state: &AppState, sc: &Scratch, key: &str) -> Result<(), String> {
    let (now, then) = (paint_hash(state)?, remembered(sc, key)?);
    if now == then { Ok(()) } else { Err(format!("the painted pixels differ from \"{key}\" ({now:x} vs {then:x})")) }
}

/// The paint layer of set 0 (the layer above Base).
pub(super) fn paint_layer(state: &AppState) -> Result<&PixelBuf, String> {
    let set = &doc_of(state)?.sets[0];
    match set.layers.get(1).map(|l| &l.kind) {
        Some(LayerKind::Raster(buf)) => Ok(buf),
        _ => Err("set 0 has no raster paint layer".to_owned()),
    }
}

/// Alpha of the paint layer at a UV position.
pub(super) fn alpha_at(state: &AppState, uv: [f32; 2]) -> Result<u8, String> {
    let buf = paint_layer(state)?;
    let (x, y) = ((uv[0] * buf.width as f32) as u32, (uv[1] * buf.height as f32) as u32);
    Ok(buf.data[((y * buf.width + x) * 4 + 3) as usize])
}

/// Painted rows (alpha >= 128) of the column at `u`.
pub(super) fn column_height(state: &AppState, u: f32) -> Result<u32, String> {
    let buf = paint_layer(state)?;
    let x = ((u * buf.width as f32) as u32).min(buf.width - 1);
    Ok((0..buf.height).filter(|&y| buf.data[((y * buf.width + x) * 4 + 3) as usize] >= 128).count() as u32)
}

/// The 2D ramp: the column near 20 % is less than half as tall as near 90 %; at the end it is the full 40 texels.
pub(super) fn assert_ramp(state: &AppState, _: &Scratch) -> Result<(), String> {
    let at = |f: f32| column_height(state, RAMP_U[0] + (RAMP_U[1] - RAMP_U[0]) * f);
    let (h20, h90, end) = (at(0.2)?, at(0.9)?, at(1.0)?);
    if h20 == 0 || h20 * 2 >= h90 {
        return Err(format!("column heights {h20} at 20 %, {h90} at 90 %"));
    }
    if end.abs_diff(SIZE as u32) > 3 {
        return Err(format!("the end is {end} texels tall, not {SIZE} +- 3"));
    }
    log::info!("pen ramp: column heights {h20} (20 %), {h90} (90 %), {end} (end)");
    Ok(())
}
