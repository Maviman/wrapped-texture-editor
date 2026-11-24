//! Dialogs, the Preview option strip and end-to-end checks on a real `WteApp` (headless wgpu device): every frame
//! runs `App::logic` + `App::ui` exactly as eframe does, so a click goes through the panels / dialogs, dispatch and
//! the jobs. Ignored by default (they need a GPU): `cargo test -p wte_app --offline -- --ignored control_sweep`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use eframe::{egui_wgpu, wgpu};
use wte_core::{FlatFormat, TexelMap};

use super::*;
use crate::app::WteApp;
use crate::gpu::Gpu;
use crate::state::{Backdrop, Dialog, ImportDialogState, SetChoiceUi, ViewId, ViewMode};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

fn headless_gpu() -> Gpu {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("no wgpu adapter");
    let base = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("control_sweep"),
        required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..base },
        ..Default::default()
    }))
    .expect("no wgpu device");
    let egui_renderer = egui_wgpu::Renderer::new(&device, Gpu::COLOR_FORMAT, egui_wgpu::RendererOptions::default());
    Gpu {
        max_side: device.limits().max_texture_dimension_2d.min(8192),
        msaa: crate::gpu::msaa_for(adapter.get_info().backend),
        device,
        queue,
        egui_renderer: std::sync::Arc::new(egui::mutex::RwLock::new(egui_renderer)),
    }
}

/// The whole app, driven like eframe drives it.
pub(crate) struct FullApp {
    pub h: Harness,
    pub app: WteApp,
    frame: eframe::Frame,
}

impl FullApp {
    fn new(kind: Option<SampleKind>) -> FullApp {
        let h = Harness::new(1280.0, 800.0);
        let app = WteApp::for_tests(h.ctx.clone(), headless_gpu());
        let mut full = FullApp { h, app, frame: eframe::Frame::_new_kittest() };
        if let Some(kind) = kind {
            crate::io_flows::install_document(&mut full.app, Document::sample(kind), None, false);
        }
        full.idle(3);
        full
    }

    fn state(&self) -> &AppState {
        &self.app.state
    }

    /// Frames until no job runs any more (at most 60 s).
    fn wait_jobs(&mut self) {
        let start = Instant::now();
        self.idle(2);
        while self.app.jobs.any_running() {
            assert!(start.elapsed() < Duration::from_secs(60), "a job did not finish");
            std::thread::sleep(Duration::from_millis(20));
            self.idle(1);
        }
        self.idle(3);
    }

    fn dialog_name(&self) -> &'static str {
        match &self.app.state.dialog {
            Dialog::None => "None",
            Dialog::Import(_) => "Import",
            Dialog::Unsaved { .. } => "Unsaved",
            Dialog::Export(_) => "Export",
            Dialog::Message { .. } => "Message",
            Dialog::About => "About",
            Dialog::Shortcuts => "Shortcuts",
            Dialog::CloseOpenings(_) => "CloseOpenings",
        }
    }

    fn export_state(&self) -> &crate::state::ExportDialogState {
        match &self.app.state.dialog {
            Dialog::Export(st) => st,
            _ => panic!("the Export dialog is not open but {}", self.dialog_name()),
        }
    }

    /// Opens a menu and clicks an entry (the entries are looked up among the widgets the popup added).
    fn menu(&mut self, path: &[&str]) {
        let before = self.h.nodes.clone();
        self.click(path[0]);
        self.idle(1);
        for sub in &path[1..path.len() - 1] {
            let p = in_new(&self.h, &before, sub).rect.center();
            self.hover(p);
            self.idle(2);
        }
        let item = in_new(&self.h, &before, path[path.len() - 1]);
        assert!(!item.disabled, "{path:?} is disabled");
        self.click_at(item.rect.center());
        self.idle(2);
    }
}

fn in_new(h: &Harness, before: &[Node], text: &str) -> Node {
    let nodes = h.new_since(before);
    let sub = Harness { nodes, ..Harness::new(1.0, 1.0) };
    sub.menu_item(text)
}

impl Drive for FullApp {
    fn h(&mut self) -> &mut Harness {
        &mut self.h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let FullApp { h, app, frame } = self;
        h.pass(events, |ui| {
            eframe::App::logic(app, ui.ctx(), frame);
            eframe::App::ui(app, ui, frame);
        });
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/review2/control-sweep").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs a GPU"]
fn about_shortcuts_and_message_dialogs_close_by_button_backdrop_and_esc() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    a.menu(&["Help", "About"]);
    assert_eq!(a.dialog_name(), "About");
    a.click("Keyboard Shortcuts...");
    a.idle(12); // the list's scroll area grows to its height over the first frames
    assert_eq!(a.dialog_name(), "Shortcuts");
    assert!(a.h.has("Navigate"), "the shortcut list is shown");
    a.click("Close");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");

    a.menu(&["Help", "About"]);
    a.click("Close");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");

    // Backdrop click and Esc.
    a.menu(&["Help", "Shortcuts"]);
    a.idle(12);
    assert_eq!(a.dialog_name(), "Shortcuts");
    a.click_at(pos2(20.0, 780.0));
    a.idle(1);
    assert_eq!(a.dialog_name(), "None", "a click outside closes the shortcut list");
    a.menu(&["Help", "About"]);
    a.press(Key::Escape, Modifiers::NONE);
    a.idle(1);
    assert_eq!(a.dialog_name(), "None", "Esc closes About");

    a.app.state.dialog = Dialog::Message { title: "Problem".into(), lines: vec!["line one".into()] };
    a.idle(2);
    assert!(a.h.has("line one"));
    // While a dialog is open the panels underneath take no clicks.
    let layers = a.state().doc().unwrap().sets[0].layers.len();
    let new = a.h.all("New").into_iter().find(|n| n.role == Role::Button).unwrap();
    a.click_at(new.rect.center());
    a.idle(1);
    assert_eq!(a.dialog_name(), "None", "a click outside the Message is a backdrop click and closes it");
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), layers, "the Layers footer under the modal took a click");
    a.app.state.dialog = Dialog::Message { title: "Problem".into(), lines: vec!["line one".into()] };
    a.idle(2);
    a.click("Close");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");
}

#[test]
#[ignore = "needs a GPU"]
fn unsaved_prompt_buttons() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    a.menu(&["Layer", "New Paint Layer"]);
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 3, "Layer > New Paint Layer added a layer");
    a.menu(&["File", "Open Sample", "Cube"]);
    assert_eq!(a.dialog_name(), "Unsaved");
    a.click("Cancel");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 3, "Cancel kept the document");
    a.menu(&["File", "Open Sample", "Cube"]);
    a.click("Don't Save");
    a.idle(2);
    assert_eq!(a.dialog_name(), "None");
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 2, "Don't Save opened the Cube sample");
}

#[test]
#[ignore = "needs a GPU"]
fn export_dialog_controls_and_export() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    a.menu(&["File", "Export..."]);
    assert_eq!(a.dialog_name(), "Export");

    // Folder: type the path.
    let dir = scratch("export");
    let folder = a.h.in_row("Folder", Role::TextInput, 0);
    a.click_at(folder.rect.center());
    a.press(Key::A, Modifiers::COMMAND);
    a.type_text(&dir.to_string_lossy());
    a.click_at(a.h.get("Base name").rect.center()); // the label: leaves the field
    assert_eq!(a.export_state().options.dir, dir);
    // Base name.
    let base = a.h.in_row("Base name", Role::TextInput, 0);
    a.click_at(base.rect.center());
    a.press(Key::A, Modifiers::COMMAND);
    a.type_text("ball");
    assert_eq!(a.export_state().options.base_name, "ball");

    // Format combo: JPEG; the quality slider and the matte button appear.
    let before = a.h.nodes.clone();
    let combo = a.h.all("PNG").into_iter().find(|n| n.role == Role::ComboBox).expect("format combo");
    a.click_at(combo.rect.center());
    a.idle(1);
    let jpeg = a.h.new_since(&before).into_iter().find(|n| n.text() == "JPEG").expect("JPEG in the format popup");
    a.click_at(jpeg.rect.center());
    a.idle(2);
    assert!(matches!(a.export_state().options.flat, Some(FlatFormat::Jpeg { .. })), "{:?}", a.export_state().options.flat);
    let quality = a.h.in_row("Quality", Role::Slider, 0);
    let y = quality.rect.center().y;
    a.drag(&[pos2(quality.rect.center().x, y), pos2(quality.rect.min.x + 20.0, y), pos2(quality.rect.min.x + 10.0, y)]);
    let Some(FlatFormat::Jpeg { quality: q, .. }) = a.export_state().options.flat else { panic!() };
    assert!(q < 60, "quality slider: {q}");
    // Matte colour popup inside the modal.
    let before = a.h.nodes.clone();
    let well = a.h.in_row("Matte", Role::ColorWell, 0);
    a.click_at(well.rect.center());
    a.idle(1);
    let square = Harness::picker_square(&a.h.new_since(&before));
    a.click_at(square.rect.min + square.rect.size() * vec2(0.9, 0.9));
    a.idle(1);
    assert_eq!(a.dialog_name(), "Export", "a click in the matte popup does not close the dialog");
    a.click_at(a.h.get("Quality").rect.center());
    a.idle(1);
    assert_ne!(a.export_state().jpeg_matte, [255, 255, 255], "matte picked");

    // Checkboxes, and the Layers choice "All" (hidden layers too).
    for label in ["OpenRaster (.ora)", "Photoshop (.psd)", "All"] {
        a.click(label);
    }
    let o = &a.export_state().options;
    assert!(o.ora && o.psd && o.include_hidden && o.layers.is_none());
    assert!(a.h.get("Selected (1)").toggled != Some(true), "one layer selected, not chosen");
    // Per-layer PNGs: off, then on again (the export below checks every per-layer PNG against its layer).
    a.click("Per-layer PNGs (stamps and text as separate files)");
    assert!(!a.export_state().options.per_layer_png);
    a.click("Per-layer PNGs (stamps and text as separate files)");
    assert!(a.export_state().options.per_layer_png);
    // Seam padding: drag.
    let pad = a.h.in_row("Seam padding", Role::SpinButton, 0);
    let c = pad.rect.center();
    a.drag(&[c, c + vec2(10.0, 0.0), c + vec2(30.0, 0.0)]);
    assert!(a.export_state().options.dilate_px > 0, "seam padding dragged");

    // The embedded preview: Unlit, and a drag orbits the preview camera.
    a.click("Unlit");
    assert!(!a.state().preview.lit);
    let rect = a.state().view_rects[&ViewId::ExportPreview];
    let yaw = a.state().session.as_ref().unwrap().preview_camera.yaw;
    a.drag(&[rect.center(), rect.center() + vec2(20.0, 0.0), rect.center() + vec2(60.0, 0.0)]);
    assert_ne!(a.state().session.as_ref().unwrap().preview_camera.yaw, yaw, "the export preview orbits");
    assert_eq!(a.dialog_name(), "Export");
    a.click("Lit");
    assert!(a.state().preview.lit);
    a.click("Show preview");
    assert!(!a.export_state().show_preview);
    a.click("Show preview");

    // Export writes exactly the planned files; a second Export asks before replacing.
    let options = a.export_state().options.clone();
    let snapshot = a.state().doc().unwrap().snapshot();
    let mut planned: Vec<String> =
        wte_core::export::plan_contents(&snapshot, &options).into_iter().map(|f| f.name).collect();
    a.click("Export");
    a.wait_jobs();
    let mut written: Vec<String> = std::fs::read_dir(&dir)
        .expect("export folder")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    planned.sort();
    assert_eq!(written, planned, "exactly the planned files");
    assert!(written.iter().any(|n| n.ends_with(".jpg")), "{written:?}");
    // The PSD holds every layer of the set (hidden ones too) and the per-layer PNGs are the layers' pixels.
    let set = &snapshot.sets[0];
    let psd_name = written.iter().find(|n| n.ends_with(".psd")).unwrap();
    let psd = wte_core::export::read_psd(&std::fs::read(dir.join(psd_name)).unwrap()).unwrap();
    assert_eq!(psd.layers.len(), set.layers.len());
    for (i, layer) in set.layers.iter().enumerate() {
        let name = wte_core::export::layer_name("ball", &set.name, false, i, layer.kind_name(), &layer.name);
        let png = wte_core::image_io::decode_image(&std::fs::read(dir.join(&name)).unwrap(), Some(&name), 16384).unwrap().pixels;
        let wte_core::LayerKind::Raster(buf) = &layer.kind else { panic!("the sample has paint layers only") };
        let dense = if buf.width == set.width { (**buf).clone() } else { wte_core::PixelBuf::new(set.width, set.height) };
        assert!(png == dense, "{name}: the layer's pixels");
    }
    assert_eq!(a.dialog_name(), "None", "a finished export closes the dialog");
    // Reopened, the dialog keeps the last folder; the other options start from the defaults again. Export those
    // twice: the second time the files exist and the confirm step comes first.
    a.menu(&["File", "Export..."]);
    assert_eq!(a.export_state().options.dir, dir);
    a.click("Export");
    a.wait_jobs();
    a.menu(&["File", "Export..."]);
    a.idle(1);
    a.click("Export");
    a.idle(2);
    assert!(a.export_state().confirm_replace.is_some(), "existing files: confirm step");
    a.click("Cancel");
    a.idle(2);
    assert!(a.export_state().confirm_replace.is_none());
    assert_eq!(a.dialog_name(), "Export");
    a.click("Export");
    a.idle(2);
    a.click("Replace");
    a.wait_jobs();
    a.menu(&["File", "Export..."]);
    a.click("Cancel");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The zip package and the layer selection through the dialog (EXPORT_SELECT_PLAN 6.2): "Pack into one .zip file"
/// plus Export writes one zip whose entries are the listed files; Export again lists the zip in the confirm step; the
/// dialog opened for two layers chooses "Selected (2)", lists two files and exports exactly those.
#[test]
#[ignore = "needs a GPU"]
fn export_dialog_zip_package_and_selected_layers() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    let dir = scratch("export_zip");
    a.menu(&["File", "Export..."]);
    if let Dialog::Export(st) = &mut a.app.state.dialog {
        st.options.dir = dir.clone();
        st.options.base_name = "ball".to_owned();
    }
    a.idle(1);
    a.click("Pack into one .zip file");
    a.idle(1);
    assert_eq!(a.export_state().options.package, wte_core::ExportPackage::Zip);
    let header = a.h.nodes.iter().find(|n| n.text().starts_with("1 file will be written: ball.zip")).cloned();
    assert!(header.is_some(), "the list names the zip");
    let snapshot = a.state().doc().unwrap().snapshot();
    let entries: Vec<String> =
        wte_core::export::plan_contents(&snapshot, &a.export_state().options).into_iter().map(|f| f.name).collect();
    for name in &entries {
        assert!(a.h.has(name), "{name} listed");
    }
    a.click("Export");
    a.wait_jobs();
    assert_eq!(a.dialog_name(), "None");
    let listing: Vec<String> =
        std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(listing, ["ball.zip"]);
    let package = wte_core::export::read_package(&std::fs::read(dir.join("ball.zip")).unwrap()).unwrap();
    assert_eq!(package.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), entries);
    let flat = wte_core::image_io::decode_image(&package[0].1, Some("ball.png"), 16384).unwrap().pixels;
    assert!(&flat == a.state().doc().unwrap().composite(TextureSetId(0)).unwrap(), "the flattened entry is the composite");
    assert!(a.state().toasts.visible_labels().iter().any(|l| l.starts_with("Exported ball.zip (")), "{:?}", a.state().toasts.visible_labels());
    // Again: the zip exists, so the confirm step lists it.
    a.menu(&["File", "Export..."]);
    if let Dialog::Export(st) = &mut a.app.state.dialog {
        st.options.base_name = "ball".to_owned();
    }
    a.idle(1);
    a.click("Pack into one .zip file");
    a.idle(2);
    assert!(a.h.has("will be replaced"));
    a.click("Export");
    a.idle(2);
    assert_eq!(a.export_state().confirm_replace.as_deref(), Some(&[dir.join("ball.zip")][..]));
    a.click("Cancel");
    a.idle(1);
    a.click("Cancel");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");

    // Opened for two layers: "Selected (2)" is chosen and two files are listed.
    let layers: Vec<wte_core::LayerId> = a.state().doc().unwrap().sets[0].layers.iter().map(|l| l.id).collect();
    a.app.state.push(AppAction::ShowExportLayers(layers.clone()));
    a.idle(3);
    assert_eq!(a.export_state().options.layers.as_deref(), Some(&layers[..]));
    assert_eq!(a.h.get("Selected (2)").toggled, Some(true));
    assert!(a.h.nodes.iter().any(|n| n.text() == "2 files will be written"), "two files listed");
    let sel_dir = scratch("export_selected");
    if let Dialog::Export(st) = &mut a.app.state.dialog {
        st.options.dir = sel_dir.clone();
    }
    a.idle(1);
    a.click("Export");
    a.wait_jobs();
    let mut listing: Vec<String> =
        std::fs::read_dir(&sel_dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    listing.sort();
    let name = a.state().doc().unwrap().name.clone();
    let base = wte_core::export::sanitize_file_stem(&name);
    assert_eq!(listing, [format!("{base}_L00_paint_Base.png"), format!("{base}_L01_paint_Paint 1.png")]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&sel_dir);
}

/// fix_export item 9: replacing an existing zip through the app's confirm step: Replace writes the new package over
/// the old file; no temp file is left; the toast names the zip.
#[test]
#[ignore = "needs a GPU"]
fn export_dialog_replaces_a_zip_through_the_confirm() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    let dir = scratch("export_replace");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ball.zip"), b"an old package").unwrap();
    a.menu(&["File", "Export..."]);
    if let Dialog::Export(st) = &mut a.app.state.dialog {
        st.options.dir = dir.clone();
        st.options.base_name = "ball".to_owned();
    }
    a.idle(1);
    a.click("Pack into one .zip file");
    a.idle(2);
    assert!(a.h.has("will be replaced"));
    a.click("Export");
    a.idle(2);
    assert_eq!(a.export_state().confirm_replace.as_deref(), Some(&[dir.join("ball.zip")][..]));
    assert!(a.h.has("Replace existing files?"));
    a.click("Replace");
    a.wait_jobs();
    assert_eq!(a.dialog_name(), "None");
    let listing: Vec<String> =
        std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(listing, ["ball.zip"], "no temp file or stage folder");
    let bytes = std::fs::read(dir.join("ball.zip")).unwrap();
    assert_ne!(bytes, b"an old package");
    let entries = wte_core::export::read_package(&bytes).unwrap();
    assert!(entries.iter().any(|(n, _)| n == "ball.png"), "{:?}", entries.iter().map(|(n, _)| n).collect::<Vec<_>>());
    assert!(a.state().toasts.visible_labels().iter().any(|l| l.starts_with("Exported ball.zip (")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// fix_export item 9: a zip export cancelled with the progress dialog's Cancel while it packs: no zip, no stage folder,
/// no temp file (the folder it created is removed again), "Export cancelled", the document unchanged. Twelve UV stamps
/// of noise make the per-layer files and the PSD large enough that packing takes a while.
#[test]
#[ignore = "needs a GPU"]
fn a_zip_export_cancelled_while_packing_leaves_nothing() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    {
        let doc = a.app.state.doc_mut().unwrap();
        let (w, h) = (doc.sets[0].width, doc.sets[0].height);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut noise = vec![0u8; (w * h * 4) as usize];
        for px in noise.chunks_mut(4) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            px.copy_from_slice(&[seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]);
        }
        let png = wte_core::encode_png(w, h, &noise, wte_core::PngLevel::Fast).unwrap();
        let blob = doc.add_image_blob("noise.png", std::sync::Arc::from(png)).unwrap();
        for i in 0..12 {
            let placement = wte_core::Placement::Uv(wte_core::UvPlacement {
                set: TextureSetId(0),
                center_px: glam::Vec2::new(w as f32 / 2.0, h as f32 / 2.0),
                size_px: glam::Vec2::new(w as f32, h as f32),
                rotation: 0.0,
                flip_x: i % 2 == 1,
                flip_y: i % 3 == 1,
                clip_to_islands: false,
            });
            doc.add_stamp(&format!("Noise {i}"), wte_core::StampSource::Image { blob }, placement, TextureSetId(0)).unwrap();
        }
    }
    a.idle(3);
    let revision = a.state().doc().unwrap().revision();
    let root = scratch("export_cancel_packing");
    let dir = root.join("out");
    let opts = wte_core::ExportOptions {
        dir: dir.clone(),
        base_name: "ball".to_owned(),
        sets: None,
        flat: Some(FlatFormat::Png(wte_core::PngLevel::Balanced)),
        per_layer_png: true,
        ora: true,
        psd: true,
        include_hidden: false,
        dilate_px: 0,
        layers: None,
        package: wte_core::ExportPackage::Zip,
    };
    a.app.state.push(AppAction::RunExport(opts));
    a.idle(1);
    assert!(a.app.jobs.modal_running(), "the export started");
    let start = Instant::now();
    let mut seen = false;
    let mut stages: Vec<String> = Vec::new();
    while a.app.jobs.any_running() {
        assert!(start.elapsed() < Duration::from_secs(120), "the export did not end");
        // Read the job's progress directly (between frames), then draw a frame.
        if let Some(b) = a.app.jobs.busy() {
            if stages.last() != Some(&b.stage) {
                stages.push(b.stage.clone());
            }
            if b.modal && b.stage == "Packing" {
                seen = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let toasts = a.state().toasts.visible_labels();
    assert!(seen, "the export ended before it was seen packing: stages {stages:?}, toasts {toasts:?} after {:?}", start.elapsed());
    // The progress dialog (it fades in over a frame or two), then its Cancel button.
    a.idle(2);
    assert!(a.app.jobs.modal_running(), "the export ended while the progress dialog appeared");
    assert!(a.h.has("Exporting"), "the progress dialog is shown");
    a.click("Cancel");
    a.wait_jobs();
    assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "something is left in {}", dir.display());
    assert!(!dir.exists(), "the folder the export created is removed again");
    assert!(a.state().toasts.visible_labels().iter().any(|l| l == "Export cancelled"), "{:?}", a.state().toasts.visible_labels());
    assert_eq!(a.dialog_name(), "None");
    assert_eq!(a.state().doc().unwrap().revision(), revision);
    let _ = std::fs::remove_dir_all(&root);
}

/// Regression test of finding `esc-in-popup-cancels`, Export dialog: Esc pressed to close the format drop-down closes
/// only the drop-down (it used to close the whole dialog, losing the chosen options); the next Esc closes the dialog.
#[test]
#[ignore = "needs a GPU"]
fn escape_in_the_export_format_popup_keeps_the_dialog() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    a.menu(&["File", "Export..."]);
    let combo = a.h.all("PNG").into_iter().find(|n| n.role == Role::ComboBox).expect("format combo");
    a.click_at(combo.rect.center());
    a.idle(1);
    assert!(egui::Popup::is_any_open(&a.h.ctx));
    a.press(Key::Escape, Modifiers::NONE);
    a.idle(1);
    assert_eq!(a.dialog_name(), "Export", "Esc meant for the drop-down closed the Export dialog");
    assert!(!egui::Popup::is_any_open(&a.h.ctx), "Esc closed the drop-down");
    a.press(Key::Escape, Modifiers::NONE);
    a.idle(1);
    assert_eq!(a.dialog_name(), "None", "the second Esc closes the dialog");
}

fn import_dialog_state() -> ImportDialogState {
    let doc = Document::sample(SampleKind::Sphere);
    let model = wte_core::import::import_from_bundle(&doc.source, &doc.import_options, true, &wte_core::NoProgress).unwrap();
    let sets = model.mesh.sets.len();
    let uv_thumbs = (0..sets).map(|s| wte_core::uv_layout_image(&model.mesh, TextureSetId(s as u32), 64)).collect();
    let uv_stats = (0..sets).map(|s| TexelMap::build(&model.mesh, TextureSetId(s as u32), 64, 64, 0).stats).collect();
    ImportDialogState {
        path: PathBuf::from(r"E:\models\sphere.obj"),
        model: Some(model),
        choices: vec![SetChoiceUi::Detected; sets],
        uv_thumbs,
        uv_stats,
        fallback_uvs: false,
        loading: false,
        error: None,
    }
}

#[test]
#[ignore = "needs a GPU"]
fn import_dialog_choices_and_create() {
    let mut a = FullApp::new(None);
    a.app.state.dialog = Dialog::Import(Box::new(import_dialog_state()));
    a.idle(2);
    let choice = |a: &FullApp| match &a.app.state.dialog {
        Dialog::Import(st) => st.choices[0].clone(),
        _ => panic!("the Import dialog closed"),
    };
    a.click("Blank");
    a.idle(1);
    assert!(matches!(choice(&a), SetChoiceUi::Blank { .. }), "{:?}", choice(&a));
    // Side combo: 512.
    let before = a.h.nodes.clone();
    let combo = a.h.nodes.iter().find(|n| n.role == Role::ComboBox && n.value.contains(" x ")).expect("side combo").clone();
    a.click_at(combo.rect.center());
    a.idle(1);
    let item = a.h.new_since(&before).into_iter().find(|n| n.text() == "512 x 512").expect("512 in the popup");
    a.click_at(item.rect.center());
    a.idle(1);
    assert!(matches!(choice(&a), SetChoiceUi::Blank { side: 512, .. }), "{:?}", choice(&a));
    // Fill colour.
    let before = a.h.nodes.clone();
    let well = a.h.nodes.iter().find(|n| n.role == Role::ColorWell).expect("fill colour").clone();
    a.click_at(well.rect.center());
    a.idle(1);
    let square = Harness::picker_square(&a.h.new_since(&before));
    a.click_at(square.rect.min + square.rect.size() * vec2(0.9, 0.9));
    a.click_at(a.h.get("Fill").rect.center());
    a.idle(1);
    assert!(matches!(choice(&a), SetChoiceUi::Blank { fill, .. } if fill != [255, 255, 255, 255]), "{:?}", choice(&a));
    // Back to Detected, then Blank again and Create.
    a.click_at(a.h.prefix("Detected:").rect.center());
    a.idle(1);
    assert!(matches!(choice(&a), SetChoiceUi::Detected));
    a.click("Blank");
    a.idle(1);
    a.click("Create");
    a.wait_jobs();
    assert_eq!(a.dialog_name(), "None");
    let doc = a.state().doc().expect("Create built the document");
    assert_eq!(doc.sets[0].width, wte_core::DEFAULT_BLANK_SIDE, "the blank base texture");

    // Cancel closes.
    a.app.state.dialog = Dialog::Import(Box::new(import_dialog_state()));
    a.idle(2);
    a.click("Cancel");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");
}

#[test]
#[ignore = "needs a GPU"]
fn preview_option_strip() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    let strip = |a: &FullApp, label: &str| {
        a.h.all(label).into_iter().find(|n| n.rect.min.x > 44.0 && n.rect.max.x < 930.0 && n.rect.min.y > 24.0).unwrap()
    };
    a.click_at(strip(&a, "Split").rect.center());
    a.idle(1);
    assert_eq!(a.state().view_mode, ViewMode::Split);
    a.click_at(strip(&a, "Preview").rect.center());
    a.idle(2);
    assert_eq!(a.state().view_mode, ViewMode::Preview);
    a.click_at(strip(&a, "Unlit").rect.center());
    assert!(!a.state().preview.lit);
    a.click_at(strip(&a, "Turntable").rect.center());
    assert!(a.state().preview.turntable);
    let speed = a.state().preview.turntable_speed;
    let slider = a.h.nodes.iter().find(|n| n.role == Role::Slider && n.rect.min.x > 44.0 && n.rect.max.x < 930.0).unwrap().clone();
    let y = slider.rect.center().y;
    a.drag(&[pos2(slider.rect.center().x, y), pos2(slider.rect.center().x + 10.0, y), pos2(slider.rect.max.x - 3.0, y)]);
    assert_ne!(a.state().preview.turntable_speed, speed);
    let yaw = a.state().session.as_ref().unwrap().preview_camera.yaw;
    a.idle(5);
    assert_ne!(a.state().session.as_ref().unwrap().preview_camera.yaw, yaw, "the turntable turns");
    let before = a.h.nodes.clone();
    let combo = a.h.all("Gradient").into_iter().find(|n| n.role == Role::ComboBox).unwrap();
    a.click_at(combo.rect.center());
    a.idle(1);
    let dark = a.h.new_since(&before).into_iter().find(|n| n.text() == "Dark").expect("Dark in the popup");
    a.click_at(dark.rect.center());
    a.idle(1);
    assert_eq!(a.state().preview.backdrop, Backdrop::Dark);
    a.click_at(strip(&a, "Nearest").rect.center());
    assert!(a.state().preview.nearest);
    a.click_at(strip(&a, "Linear").rect.center());
    assert!(!a.state().preview.nearest);
    a.click_at(strip(&a, "Export...").rect.center());
    a.idle(2);
    assert_eq!(a.dialog_name(), "Export");
    a.click("Cancel");
    a.idle(1);
    a.click_at(strip(&a, "Back").rect.center());
    a.idle(1);
    assert_eq!(a.state().view_mode, ViewMode::Split, "Back returns to the last editing mode");
}

#[test]
#[ignore = "needs a GPU"]
fn end_to_end_menu_toolbar_and_keys() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    a.menu(&["View", "Wireframe"]);
    assert!(a.state().flags.wireframe);
    a.press(Key::W, Modifiers::NONE);
    assert!(!a.state().flags.wireframe, "W toggles the wireframe back");
    a.menu(&["Layer", "New Paint Layer"]);
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 3);
    a.menu(&["Edit", "Undo"]);
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 2);
    a.press(Key::Y, Modifiers::COMMAND);
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), 3, "Ctrl+Y redoes");
    let eraser = a.h.prefix("Eraser (").rect.center();
    a.click_at(eraser);
    assert_eq!(a.state().tool, crate::state::ToolKind::Eraser);
    // The Layers row controls under the full app (eye of the active layer).
    let eye = a.app.state.panel_rects[&crate::state::PanelHit::LayerEye(a.state().doc().unwrap().sets[0].layers[2].id)].center();
    a.click_at(eye);
    assert!(!a.state().doc().unwrap().sets[0].layers[2].visible, "the eye hides the layer");
}


#[test]
#[ignore = "needs a GPU"]
fn progress_dialog_cancel_stops_the_modal_job_and_blocks_the_ui_meanwhile() {
    let mut a = FullApp::new(Some(SampleKind::Sphere));
    let ctx = a.h.ctx.clone();
    a.app.jobs.spawn(&ctx, "Exporting", true, |p| {
        let start = Instant::now();
        while !wte_core::Progress::cancelled(p) && start.elapsed() < Duration::from_secs(30) {
            wte_core::Progress::report(p, 0.25, "Writing");
            std::thread::sleep(Duration::from_millis(5));
        }
        crate::jobs::JobOutput::Exported { result: Err("cancelled".into()), dir: PathBuf::new() }
    });
    a.idle(3);
    assert!(a.h.has("Exporting"), "the progress dialog shows the job");
    // Underneath, menus, the toolbar and keys are blocked.
    let layers = a.state().doc().unwrap().sets[0].layers.len();
    let new = a.h.all("New").into_iter().find(|n| n.role == Role::Button).unwrap();
    a.click_at(new.rect.center());
    a.press(Key::N, Modifiers::COMMAND.plus(Modifiers::SHIFT));
    a.press(Key::Escape, Modifiers::NONE);
    a.idle(1);
    assert_eq!(a.state().doc().unwrap().sets[0].layers.len(), layers);
    assert!(a.app.jobs.modal_running(), "Esc does not cancel a modal job");
    a.click("Cancel");
    a.wait_jobs();
    assert!(!a.app.jobs.any_running(), "Cancel stopped the job");
    assert_eq!(a.dialog_name(), "None", "a cancelled job is no failure: no message");
}

#[test]
#[ignore = "needs a GPU"]
fn export_dialog_at_the_smallest_window_and_with_several_sets() {
    let mut a = FullApp::new(Some(SampleKind::TwoMaterial));
    a.h.size = vec2(900.0, 600.0);
    a.idle(3);
    a.menu(&["File", "Export..."]);
    a.idle(3);
    let screen = Rect::from_min_size(Pos2::ZERO, a.h.size);
    for label in ["Export", "Cancel", "Show preview", "Visible", "All", "Pack into one .zip file", "Lit", "Unlit"] {
        let node = a.h.get(label);
        assert!(screen.contains_rect(node.rect), "{label} at {:?} is off screen", node.rect);
    }
    // One checkbox per set; unticking every set disables Export (with a folder set, that is the reason shown).
    if let Dialog::Export(st) = &mut a.app.state.dialog {
        st.options.dir = scratch("export_sets");
    }
    a.idle(1);
    let names: Vec<String> = a.state().doc().unwrap().sets.iter().map(|s| s.name.clone()).collect();
    a.click(&names[1]);
    assert_eq!(a.export_state().options.sets, Some(vec![TextureSetId(0)]));
    a.click(&names[0]);
    a.idle(1);
    assert_eq!(a.export_state().options.sets, Some(vec![]));
    assert!(a.h.get("Export").disabled, "no set: Export is disabled");
    assert!(a.h.has("Choose at least one texture set"));
    a.click(&names[0]);
    a.click(&names[1]);
    assert_eq!(a.export_state().options.sets, None, "all sets again");
    // Unticking every kind of file disables Export too.
    a.click("Flattened texture");
    a.click("Per-layer PNGs (stamps and text as separate files)");
    a.idle(1);
    assert!(a.h.get("Export").disabled);
    a.click("Cancel");
    a.idle(1);
    assert_eq!(a.dialog_name(), "None");
}

/// objects-tabs (plan 4.7): H during a held brush stroke ends the stroke first by committing it (one undo entry, the
/// stroke) and then hides the selected object, which adds no undo entry, no revision and no modified mark.
#[test]
#[ignore = "needs a GPU"]
fn hiding_during_a_held_stroke_commits_the_stroke_first() {
    use crate::state::ToolKind;
    use crate::tools::{Surface, ViewXform};
    let mut a = FullApp::new(Some(SampleKind::Objects));
    a.app.state.tool = ToolKind::Brush;
    a.app.state.session.as_mut().unwrap().selected_objects = [1].into();
    a.idle(2);
    let xf = ViewXform::for_view(a.state(), ViewId::Main3D).expect("the 3D view is laid out");
    let doc = a.state().doc().unwrap();
    let cube = doc.mesh.objects[1].bounds.center();
    let at = xf.world_to_screen(cube).expect("the cube is in front of the camera");
    let Surface::Model(hit) = xf.surface_at(doc, TextureSetId(0), at) else { panic!("nothing under the cube's centre") };
    assert_eq!(doc.mesh.tri_object_of(hit.tri), 1, "the cube is under its centre");
    let undo = doc.history.undo_len();

    // Press and move: the stroke is held.
    a.hover(at);
    a.frame(vec![Harness::button(at, egui::PointerButton::Primary, true, Modifiers::NONE)]);
    for k in 1..=6 {
        let p = at + egui::vec2(3.0 * k as f32, 0.0);
        a.h.pointer = p;
        a.frame(vec![Event::PointerMoved(p)]);
    }
    assert!(a.app.tools.is_busy(), "the stroke is held");
    assert!(a.state().doc().unwrap().stroke_active());
    let revision_before_hide = a.state().doc().unwrap().revision();

    a.press(Key::H, Modifiers::NONE);
    let doc = a.state().doc().unwrap();
    assert!(!a.app.tools.is_busy(), "the stroke was ended");
    assert!(!doc.stroke_active());
    assert_eq!(doc.hidden_objects(), [false, true, false], "the cube is hidden");
    assert_eq!(doc.history.undo_len(), undo + 1, "one undo entry: the stroke");
    assert_eq!(doc.undo_label(), Some("Brush Stroke"));
    assert!(doc.revision() > revision_before_hide, "the stroke was committed");
    let (revision, modified) = (doc.revision(), doc.is_modified());

    // The release after the hide does nothing more; Alt+H shows everything again without a history entry.
    let end = a.h.pointer;
    a.frame(vec![Harness::button(end, egui::PointerButton::Primary, false, Modifiers::NONE)]);
    a.press(Key::H, Modifiers::ALT);
    let doc = a.state().doc().unwrap();
    assert_eq!(doc.hidden_object_count(), 0);
    assert_eq!((doc.history.undo_len(), doc.revision(), doc.is_modified()), (undo + 1, revision, modified));
}

// ---------------------------------------------------------------------------------------------------------------
// The brush ring's side filter (FEATURE_BRUSH 5.11, 6.5; test A15), on the renderer alone.

/// Reads the resolved image of a view target back as tightly packed RGBA8.
fn read_target(gpu: &Gpu, target: &crate::gpu::ViewTarget) -> Vec<u8> {
    let [w, h] = target.size_px();
    let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ring_read_back"),
        size: padded as u64 * h as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        target.resolve_texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("map failed"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll failed");
    let mapped = buffer.get_mapped_range(..).expect("mapped range");
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in mapped.chunks(padded as usize).take(h as usize) {
        out.extend_from_slice(&row[..(w * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// A15 (FEATURE_BRUSH 8.2): the ring at a cube corner with a side normal differs from the one without. The ring is
/// hidden exactly on the faces the Circular brush skips (`dot(face normal, side) < -0.2`, the core's rule) and kept on
/// the others, perpendicular faces included; without a side normal it is the ring of round 3.
#[test]
#[ignore = "needs a GPU"]
fn the_ring_at_a_cube_corner_follows_the_side_normal() {
    use crate::gpu::{BrushRing, GpuScene, Renderer, View3dParams, ViewTargets};
    use crate::state::ShadingMode;
    use glam::{Mat4, Vec3};

    let gpu = headless_gpu();
    let mut renderer = Renderer::new(&gpu);
    let mut targets = ViewTargets::new(&gpu);
    let mut doc = Document::sample(SampleKind::Cube); // seam_cube(1.5): flat normals, corner at (0.75, 0.75, 0.75)
    let scene = GpuScene::new(&gpu, &renderer, &mut doc, 1);
    let (w, h) = (256u32, 256u32);
    let target = &mut targets.main3d;
    target.ensure_size(&gpu, [w, h]);

    // A camera that sees the +X, +Y and +Z faces around the corner.
    let corner = Vec3::splat(0.75);
    let eye = Vec3::new(2.6, 2.2, 3.0);
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::splat(0.35), Vec3::Y);
    let view_proj = glam::camera::rh::proj::directx::perspective(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 20.0) * view;
    let radius = 0.5;
    let base = View3dParams {
        view_proj,
        eye,
        shading: ShadingMode::Clay,
        lit: false,
        alpha_checker: false,
        matte: None,
        wireframe: false,
        seams: false,
        nearest: false,
        backdrop: Backdrop::Dark,
        brush: None,
    };
    let mut render = |side_normal: Option<Vec3>, ring: bool| {
        let brush = ring.then_some(BrushRing { center: corner, radius, hardness: 1.0, side_normal });
        renderer.render_3d(&gpu, &scene, target, &View3dParams { brush, ..base.clone() });
        read_target(&gpu, target)
    };
    let plain = render(None, false);
    let all_faces = render(None, true);
    let side_z = render(Some(Vec3::Z), true);
    let side_neg_z = render(Some(Vec3::NEG_Z), true);
    let side_unnormalised = render(Some(Vec3::new(0.0, 0.0, -7.0)), true);

    let changed = |img: &[u8]| img.chunks(4).zip(plain.chunks(4)).filter(|(a, b)| a != b).count();
    let to_px = |p: Vec3| {
        let ndc = Mat4::project_point3(&view_proj, p);
        ((ndc.x + 1.0) * 0.5 * w as f32, (1.0 - ndc.y) * 0.5 * h as f32)
    };
    // Number of pixels near the ring point `p` (a 9 x 9 window) that differ from the render without a ring.
    let ring_near = |img: &[u8], p: Vec3| {
        let (cx, cy) = to_px(p);
        let mut n = 0;
        for y in (cy as i32 - 4)..=(cy as i32 + 4) {
            for x in (cx as i32 - 4)..=(cx as i32 + 4) {
                assert!(x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h, "({x}, {y}) is off the target");
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                n += usize::from(img[i..i + 4] != plain[i..i + 4]);
            }
        }
        n
    };
    let d = radius / std::f32::consts::SQRT_2;
    let on_pos_z = corner + Vec3::new(-d, -d, 0.0);
    let on_pos_x = corner + Vec3::new(0.0, -d, -d);
    let on_pos_y = corner + Vec3::new(-d, 0.0, -d);

    let n_all = changed(&all_faces);
    assert!(n_all > 200, "the ring without a side normal shows only {n_all} pixels");
    for (face, p) in [("+Z", on_pos_z), ("+X", on_pos_x), ("+Y", on_pos_y)] {
        assert!(ring_near(&all_faces, p) > 0, "no side normal: the ring is missing on the {face} face");
    }
    // Side +Z (a hover on the front face): no visible face is turned away (the two others are perpendicular, dot 0
    // >= -0.2), so the image is the one without a filter.
    assert_eq!(side_z, all_faces, "side +Z: perpendicular faces keep the ring");
    // Side -Z: the +Z face is on the other side and loses its ring; +X and +Y keep theirs.
    assert_ne!(side_neg_z, all_faces, "A15: a side normal changes the ring image");
    assert_eq!(ring_near(&side_neg_z, on_pos_z), 0, "side -Z: the ring is still drawn on the +Z face");
    assert!(ring_near(&side_neg_z, on_pos_x) > 0, "side -Z: the ring vanished from the +X face");
    assert!(ring_near(&side_neg_z, on_pos_y) > 0, "side -Z: the ring vanished from the +Y face");
    let n_neg = changed(&side_neg_z);
    assert!(n_neg > 0 && n_neg < n_all, "side -Z: {n_neg} ring pixels against {n_all} without a filter");
    // The packing normalises: a long normal filters like the unit one.
    assert_eq!(side_unnormalised, side_neg_z);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/selftest");
    std::fs::create_dir_all(&dir).expect("create target/selftest");
    for (name, img) in [("none", &all_faces), ("side_pos_z", &side_z), ("side_neg_z", &side_neg_z)] {
        let png = wte_core::encode_png(w, h, img, wte_core::PngLevel::Fast).expect("encode png");
        std::fs::write(dir.join(format!("gpu_ring_corner_{name}.png")), png).expect("write png");
    }
    let ViewTargets { main3d, main2d, preview, export_preview } = targets;
    for t in [main3d, main2d, preview, export_preview] {
        t.destroy(&gpu);
    }
}
