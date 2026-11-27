// Owner: app-shell (A1). Drag-and-drop of ARCHITECTURE.md 8.9 and 8.17: the "Drop to open" overlay while files hover
// over the window, dropped paths as `AppAction::OpenPath` (dispatch routes them by extension), and (stickers P10a,
// STICKER_PLAN 6.1, 6.2) drops at a point from WTE's own OLE drop target: pictures, `.wtes` and `.psd` files or picture
// data land where they were let go (a stamp on the model, a layer on the sticker canvas, a stamp's row).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use eframe::egui;
use wte_core::{StampId, StampSource, StickerDoc, WtesFile};

use crate::action::{AppAction, ToastLevel};
use crate::ole::{DataKind, DragKinds, DropItem, OleEvent};
use crate::state::{AppState, Dialog, PanelHit, StampToolSource, ToolKind, ViewId, ViewMode};
use crate::tools::{Surface, Tool, ViewPointer, ViewXform};

/// Drag-and-drop state of the app (`AppState::drop_ui`).
#[derive(Default)]
pub struct DropUi {
    /// The drag over the window (from egui's file list, or from the OLE drop target with its point).
    pub hover: Option<DropHover>,
    /// The hover came from egui's `hovered_files` (cleared when that list is empty again).
    egui_hover: bool,
    /// Drops being prepared on a worker (files read, picture data converted, sticker and PSD files parsed).
    jobs: Vec<mpsc::Receiver<Prepared>>,
}

impl DropUi {
    /// True while a drop is being prepared (the self-test's idle check).
    pub(crate) fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }
}

/// A drag hovering over the window.
#[derive(Clone, Debug, PartialEq)]
pub struct DropHover {
    /// Points; `None` for egui's file list, which has no point.
    pub pos: Option<egui::Pos2>,
    pub kinds: DragKinds,
}

/// What the overlay shows for a hover (the same for both sources of hovers).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Overlay {
    /// A modal job runs: nothing may be dropped.
    Blocked,
    /// A project or model (or anything WTE does not take at a point): the full-window "Drop to open".
    Open,
    /// Pictures, stickers or PSD files: a marker on the place they would land (with a point), else "Drop to open".
    Place,
}

/// The overlay of a hover: `Open` for a project or a model (they replace the document) and for anything that is not
/// a picture, sticker or PSD; `Place` for those.
pub(crate) fn overlay_of(kinds: &DragKinds, blocked: bool) -> Overlay {
    if blocked {
        return Overlay::Blocked;
    }
    let document = kinds.extensions.iter().any(|e| e == wte_core::PROJECT_EXTENSION || MODEL_EXTENSIONS.contains(&e.as_str()));
    let placeable = kinds.picture || kinds.sticker || kinds.extensions.iter().any(|e| is_placeable_extension(e));
    if document || !placeable { Overlay::Open } else { Overlay::Place }
}

/// Where a drop lands (6.2), looked up at the drop point.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Spot {
    /// A 3D or the 2D main view at a point (the surface under it is looked up when the drop is applied: a miss is
    /// `Elsewhere`).
    View(ViewId, egui::Pos2),
    /// The sticker canvas of the Sticker view.
    Canvas(egui::Pos2),
    /// A stamp's row in the Layers list, or the selected stamp's thumbnail in the Sticker tab.
    Stamp(StampId),
    /// Anywhere else.
    Elsewhere,
}

/// The spot under `pos` (points) in the last laid-out frame.
pub(crate) fn spot_at(state: &AppState, pos: egui::Pos2) -> Spot {
    let Some(session) = state.session.as_ref() else { return Spot::Elsewhere };
    if !matches!(state.dialog, Dialog::None) {
        return Spot::Elsewhere;
    }
    let list = state.panel_rects.get(&PanelHit::LayersList);
    for (hit, rect) in &state.panel_rects {
        if !rect.contains(pos) {
            continue;
        }
        match hit {
            PanelHit::LayerRow(layer) if list.is_none_or(|l| l.contains(pos)) => {
                if let Some(stamp) = session.doc.stamp_of_layer(*layer) {
                    return Spot::Stamp(stamp);
                }
            }
            PanelHit::StickerTabThumb => {
                if let Some(stamp) = crate::sticker::selected_stamps(state).first() {
                    return Spot::Stamp(*stamp);
                }
            }
            _ => {}
        }
    }
    match state.view_mode {
        ViewMode::Sticker => match state.sticker_canvas_rect {
            Some(r) if r.contains(pos) => Spot::Canvas(pos),
            _ => Spot::Elsewhere,
        },
        ViewMode::Preview => Spot::Elsewhere,
        _ => [ViewId::Main3D, ViewId::Main2D]
            .into_iter()
            .find(|v| state.view_rects.get(v).is_some_and(|r| r.contains(pos)))
            .map_or(Spot::Elsewhere, |v| Spot::View(v, pos)),
    }
}

/// The view transform and the surface of the model or the texture under a `View` spot; `None` for a miss.
fn surface_of(state: &AppState, view: ViewId, pos: egui::Pos2) -> Option<(ViewXform, Surface)> {
    let xf = ViewXform::for_view(state, view)?;
    let session = state.session.as_ref()?;
    let surface = xf.surface_at(&session.doc, session.active_set, pos);
    (!matches!(surface, Surface::Miss)).then_some((xf, surface))
}

/// The marker's sentence for a hover of `kinds` over `spot`.
pub(crate) fn hover_text(state: &AppState, spot: Spot, kinds: &DragKinds) -> String {
    if state.session.is_none() {
        return "Open a model first".to_owned();
    }
    let sticker = kinds.sticker || kinds.extensions.first().is_some_and(|e| matches!(e.as_str(), "psd" | "psb" | wte_core::WTES_EXTENSION));
    let what = if sticker { "the sticker" } else { "the picture" };
    let more = kinds.extensions.len().saturating_sub(1);
    let tail = if more > 0 { format!(" ({more} more go to the tray)") } else { String::new() };
    let text = match spot {
        Spot::View(view, pos) if surface_of(state, view, pos).is_some() => format!("Drop to place {what} here"),
        Spot::Canvas(_) => format!("Drop to add {what} as layers of this sticker"),
        Spot::Stamp(stamp) => {
            let source = state.doc().and_then(|d| d.stamp(stamp)).map(|s| s.source.clone());
            match (source, sticker) {
                (Some(StampSource::Text(_)), _) => "A text stamp's picture cannot be replaced".to_owned(),
                (Some(StampSource::Image { .. }), false) => "Drop to replace this stamp's picture".to_owned(),
                (Some(StampSource::Sticker { .. }), false) => "Drop to add the picture as a layer of this sticker".to_owned(),
                _ => "Drop to show the sticker on this stamp".to_owned(),
            }
        }
        _ if sticker => "Drop to add the sticker to the tray".to_owned(),
        _ => "Drop to use the picture with the Stamp tool".to_owned(),
    };
    text + &tail
}

/// Draws the overlay of the hover, takes this frame's drops and starts preparing them, and hands the prepared drops to
/// dispatch (`AppAction::DropReady`). Reads both sources: the OLE queue (WTE's drop target, the self-test's drops) and
/// egui's `hovered_files` / `dropped_files` (where WTE's target is not installed).
pub fn handle(ctx: &egui::Context, state: &mut AppState) {
    let _p = crate::app::frame_profile::scope("ole.drop");
    handle_events(ctx, state, crate::ole::drain());
}

/// `handle` with the queued events given (the tests pass their own).
pub(crate) fn handle_events(ctx: &egui::Context, state: &mut AppState, events: Vec<OleEvent>) {
    if crate::ole::take_registration_lost() {
        state.toasts.warn("Dropping files on WTE does not work in this run (the drop target could not be registered)");
    }
    let (hovered, dropped) = ctx.input(|i| {
        let dropped: Vec<PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
        let hovered: Vec<String> = i.raw.hovered_files.iter().map(|f| f.path.as_deref().map(extension_of).unwrap_or_default()).collect();
        (hovered, dropped)
    });
    let ppp = ctx.pixels_per_point().max(1e-3);
    let ui = &mut state.drop_ui;
    if !hovered.is_empty() {
        ui.hover = Some(DropHover { pos: None, kinds: DragKinds { extensions: hovered, ..DragKinds::default() } });
        ui.egui_hover = true;
    } else if ui.egui_hover {
        ui.hover = None;
        ui.egui_hover = false;
    }
    let mut drops = Vec::new();
    for event in events {
        match event {
            OleEvent::Hover { pos, kinds } => {
                ui.hover = Some(DropHover { pos: Some(egui::pos2(pos[0], pos[1]) / ppp), kinds });
                ui.egui_hover = false;
            }
            OleEvent::Leave => ui.hover = None,
            OleEvent::Drop { pos, items, skipped } => {
                ui.hover = None;
                drops.push((egui::pos2(pos[0], pos[1]) / ppp, items, skipped));
            }
        }
    }
    // While a modal job runs nothing may replace the session (8.9): drops are ignored.
    let blocked = state.busy.as_ref().is_some_and(|b| b.modal);
    if let Some(hover) = state.drop_ui.hover.clone() {
        draw_overlay(ctx, state, &hover, blocked);
    }
    if !dropped.is_empty() {
        drop_paths(state, &dropped, blocked);
    }
    for (pos, items, skipped) in drops {
        start_drop(ctx, state, pos, items, skipped, blocked);
    }
    poll_jobs(state);
}

/// The overlay: "Drop to open" over the whole window (a project or model, or egui's file list without a point), or a
/// frame around the place a picture would land with a sentence beside the pointer.
fn draw_overlay(ctx: &egui::Context, state: &AppState, hover: &DropHover, blocked: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("wte_drop_overlay")));
    let overlay = overlay_of(&hover.kinds, blocked);
    if let (Overlay::Place, Some(pos)) = (overlay, hover.pos) {
        let spot = spot_at(state, pos);
        let area = match spot {
            Spot::View(view, _) => state.view_rects.get(&view).copied(),
            Spot::Canvas(_) => state.sticker_canvas_rect,
            Spot::Stamp(_) => state
                .panel_rects
                .iter()
                .find(|(hit, r)| matches!(hit, PanelHit::LayerRow(_) | PanelHit::StickerTabThumb) && r.contains(pos))
                .map(|(_, r)| *r),
            Spot::Elsewhere => None,
        };
        if let Some(area) = area {
            painter.rect_filled(area, 4.0, crate::theme::ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(area.shrink(1.0), 4.0, egui::Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
        }
        let text = hover_text(state, spot, &hover.kinds);
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(15.0), egui::Color32::WHITE);
        let content = ctx.content_rect();
        let mut at = pos + egui::vec2(18.0, 18.0);
        at.x = at.x.min(content.right() - galley.size().x - 12.0).max(content.left() + 4.0);
        at.y = at.y.min(content.bottom() - galley.size().y - 12.0).max(content.top() + 4.0);
        let bg = egui::Rect::from_min_size(at, galley.size()).expand(6.0);
        painter.rect_filled(bg, 4.0, egui::Color32::from_black_alpha(200));
        painter.galley(at, galley, egui::Color32::WHITE);
        painter.circle_stroke(pos, 6.0, egui::Stroke::new(2.0, crate::theme::ACCENT));
        return;
    }
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(170));
    painter.rect_stroke(rect.shrink(12.0), 8.0, egui::Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
    let text = if blocked { "Please wait for the running operation to finish" } else { "Drop to open" };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(28.0), egui::Color32::WHITE);
    if !blocked {
        let hint = "Models (.obj, .gltf, .glb, .fbx), projects (.wtep), images (stamp), PSD or sticker files (sticker)";
        let pos = rect.center() + egui::vec2(0.0, 32.0);
        painter.text(pos, egui::Align2::CENTER_CENTER, hint, egui::FontId::proportional(15.0), egui::Color32::LIGHT_GRAY);
    }
}

/// egui's dropped files (no point): as before P10a, every path through `plan_drop` and `OpenPath`.
fn drop_paths(state: &mut AppState, dropped: &[PathBuf], blocked: bool) {
    if blocked {
        state.toasts.warn("Please wait for the running operation to finish");
        return;
    }
    let plan = plan_drop(dropped);
    for path in dropped {
        log::info!("dropped {}", path.display());
    }
    toast_ignored(state, &plan);
    if let Some(doc) = plan.document {
        state.push(AppAction::OpenPath(doc));
    }
    for image in plan.stamps {
        state.push(AppAction::OpenPath(image));
    }
}

fn toast_ignored(state: &mut AppState, plan: &DropPlan) {
    if !plan.ignored.is_empty() {
        let names: Vec<String> = plan.ignored.iter().map(|p| file_name(p)).collect();
        state.push(AppAction::Toast(ToastLevel::Info, format!("Not opened: {} ({})", names.join(", "), plan.reason)));
    }
}

/// A drop of the OLE target at `pos` (6.2): a project or model among files opens as before; otherwise, with a project
/// open, the items are prepared on a worker (files read, picture data made a PNG, `.wtes` / `.psd` parsed) and placed
/// by `apply` at the spot found now.
fn start_drop(ctx: &egui::Context, state: &mut AppState, pos: egui::Pos2, items: Vec<DropItem>, skipped: usize, blocked: bool) {
    log::info!("drop of {} item(s) at {pos:?} ({skipped} over a size limit)", items.len());
    if skipped > 0 {
        let s = if skipped == 1 { " was" } else { "s were" };
        state.toasts.warn(format!("{skipped} dropped item{s} too large (more than 64 MB) and not taken"));
    }
    if items.is_empty() {
        return;
    }
    if blocked {
        state.toasts.warn("Please wait for the running operation to finish");
        return;
    }
    let files: Vec<PathBuf> = items.iter().filter_map(|i| if let DropItem::File(p) = i { Some(p.clone()) } else { None }).collect();
    let plan = plan_drop(&files);
    for path in &files {
        log::info!("dropped {}", path.display());
    }
    toast_ignored(state, &plan);
    if let Some(document) = plan.document {
        state.push(AppAction::OpenPath(document));
        return;
    }
    let items: Vec<DropItem> = items.into_iter().filter(|i| !matches!(i, DropItem::File(p) if plan.ignored.contains(p))).collect();
    if items.is_empty() {
        return;
    }
    if state.session.is_none() {
        state.toasts.warn("Open a model first");
        return;
    }
    let spot = spot_at(state, pos);
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let prepared = prepare(spot, items);
        let _ = tx.send(prepared);
        ctx.request_repaint();
    });
    state.drop_ui.jobs.push(rx);
}

/// Hands the drops whose preparation is done to dispatch.
fn poll_jobs(state: &mut AppState) {
    let mut done = Vec::new();
    state.drop_ui.jobs.retain(|rx| match rx.try_recv() {
        Ok(p) => {
            done.push(p);
            false
        }
        Err(mpsc::TryRecvError::Empty) => true,
        Err(mpsc::TryRecvError::Disconnected) => false,
    });
    for p in done {
        state.push(AppAction::DropReady(Arc::new(p)));
    }
}

/// One dropped thing, ready to be used on the UI thread.
pub(crate) enum Ready {
    /// An encoded picture (a file's bytes, PNG data, or picture data made a PNG); `path` for a file.
    Picture { name: String, bytes: Arc<[u8]>, path: Option<PathBuf> },
    /// A `.wtes` (a file or WTE's sticker data).
    Sticker { file: Box<WtesFile> },
    /// A read PSD.
    Psd { file: String, stem: String, stamp: wte_core::link::FileStamp, psd: Box<wte_core::psd::PsdDoc> },
}

/// A drop, prepared: where it lands, the things to use (in drop order), and the sentences of those that could not be
/// read.
pub struct Prepared {
    pub(crate) spot: Spot,
    pub(crate) ready: Vec<Ready>,
    pub(crate) failed: Vec<String>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<&str> = self
            .ready
            .iter()
            .map(|r| match r {
                Ready::Picture { .. } => "picture",
                Ready::Sticker { .. } => "sticker",
                Ready::Psd { .. } => "psd",
            })
            .collect();
        write!(f, "Prepared {{ spot: {:?}, ready: {kinds:?}, failed: {} }}", self.spot, self.failed.len())
    }
}

/// Reads and converts the items (on the worker): files by extension, picture data as is or (a DIB) made a PNG,
/// sticker data and PSDs parsed.
pub(crate) fn prepare(spot: Spot, items: Vec<DropItem>) -> Prepared {
    let mut out = Prepared { spot, ready: Vec::new(), failed: Vec::new() };
    for item in items {
        let made = match item {
            DropItem::File(path) => {
                let name = file_name(&path);
                let modified = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                std::fs::read(&path).map_err(|e| format!("Cannot open {name}: {e}")).and_then(|bytes| ready_of(&name, bytes, Some(path), modified))
            }
            DropItem::Data { name, kind: DataKind::Png | DataKind::File, bytes } => ready_of(&name, bytes, None, None),
            DropItem::Data { name, kind: DataKind::Dib, bytes } => crate::ole::formats::dib_to_pixels(&bytes)
                .and_then(|p| wte_core::encode_png(p.width, p.height, &p.data, wte_core::PngLevel::Fast).map_err(|e| e.to_string()))
                .map(|png| Ready::Picture { name, bytes: png.into(), path: None })
                .map_err(|e| format!("Cannot use the dropped picture: {e}")),
            DropItem::Data { name, kind: DataKind::Sticker, bytes } => sticker_of(&name, &bytes),
        };
        match made {
            Ok(r) => out.ready.push(r),
            Err(text) => out.failed.push(text),
        }
    }
    out
}

/// A file's (or a virtual file's) bytes by its extension.
fn ready_of(name: &str, bytes: Vec<u8>, path: Option<PathBuf>, modified: Option<std::time::SystemTime>) -> Result<Ready, String> {
    let p = Path::new(name);
    if is_psd_path(p) {
        let stamp = wte_core::link::FileStamp::of_bytes(&bytes, modified);
        let psd = crate::link::flows::parse_psd(&bytes, name)?;
        let stem = p.file_stem().map_or_else(|| "Sticker".to_owned(), |s| s.to_string_lossy().into_owned());
        return Ok(Ready::Psd { file: name.to_owned(), stem, stamp, psd: Box::new(psd) });
    }
    if is_wtes_path(p) {
        return sticker_of(name, &bytes);
    }
    Ok(Ready::Picture { name: name.to_owned(), bytes: bytes.into(), path })
}

fn sticker_of(name: &str, bytes: &[u8]) -> Result<Ready, String> {
    let file = wte_core::read_wtes(bytes).map_err(|e| format!("Cannot read {name}: {e}"))?;
    for w in &file.warnings {
        log::warn!("{name}: {w}");
    }
    Ok(Ready::Sticker { file: Box::new(file) })
}

/// Places a prepared drop (6.2), as one undo step: the first item by the table of 6.2 at its spot, every further one
/// into the tray ("N more added to the tray").
///
/// | Spot | Picture | `.wtes` / `.psd` |
/// |---|---|---|
/// | a view, the model / texture under the point | a stamp there (the Stamp tool's click) | the same as a sticker stamp |
/// | the sticker canvas | a picture layer centred at the point | its layers, centred at the point |
/// | a stamp's row or the Sticker tab's thumbnail | image stamp: its picture replaced; sticker stamp: a new top layer | the stamp shows the sticker |
/// | anywhere else, a miss | the Stamp tool's picture | a tray sticker, the Stamp tool's source |
pub(crate) fn apply(state: &mut AppState, p: &Prepared) {
    for text in &p.failed {
        state.toasts.warn(text.clone());
    }
    let Some((first, rest)) = p.ready.split_first() else { return };
    let Some(doc) = state.doc_mut() else {
        state.toasts.warn("Open a model first");
        return;
    };
    let group = doc.begin_edit_group();
    apply_first(state, p.spot, first);
    let mut to_tray = 0;
    for item in rest {
        if to_tray_sticker(state, item).is_some() {
            to_tray += 1;
        }
    }
    if let Some(doc) = state.doc_mut() {
        doc.end_edit_group(group, "Drop");
    }
    if to_tray > 0 {
        state.toasts.info(format!("{to_tray} more added to the tray"));
    }
}

/// The first item at its spot.
fn apply_first(state: &mut AppState, spot: Spot, item: &Ready) {
    match spot {
        Spot::View(view, pos) => match surface_of(state, view, pos) {
            Some((xf, surface)) => {
                if let Some((source, name)) = stamp_source(state, item) {
                    place(state, &xf, ViewPointer { view, pos, surface, pressure: 1.0, mods: egui::Modifiers::NONE }, source, name);
                }
            }
            None => elsewhere(state, item),
        },
        Spot::Canvas(pos) => {
            let at = crate::sticker::canvas::layer_point(state, pos);
            let source = if at.is_some() { sticker_doc_of(state, item) } else { None };
            match source {
                Some(source) => {
                    let label = if matches!(item, Ready::Picture { .. }) { "Add Picture Layer" } else { "Add Sticker Layers" };
                    crate::sticker::add_layers_at(state, source, at, label);
                }
                None => elsewhere(state, item),
            }
        }
        Spot::Stamp(stamp) => on_stamp(state, stamp, item),
        Spot::Elsewhere => elsewhere(state, item),
    }
}

/// The Stamp tool's source for a drop on the model: a picture's blob, or a new tray sticker.
fn stamp_source(state: &mut AppState, item: &Ready) -> Option<(StampToolSource, String)> {
    match item {
        Ready::Picture { name, bytes, .. } => {
            let blob = add_blob(state, name, bytes)?;
            Some((StampToolSource::Blob(blob), name.clone()))
        }
        _ => {
            let id = to_tray_sticker(state, item)?;
            state.sticker_ui.tray_selected = Some(id);
            let name = state.doc().and_then(|d| d.sticker(id)).map(|s| s.name.clone()).unwrap_or_default();
            Some((StampToolSource::Sticker(id), name))
        }
    }
}

/// A stamp at the drop point, exactly as a click of the Stamp tool with this source (placement, size, styles, one
/// entry); the new stamp is selected and the Transform tool takes over, as after a click.
fn place(state: &mut AppState, xf: &ViewXform, pointer: ViewPointer, source: StampToolSource, name: String) {
    state.tools.stamp.source = source;
    state.tools.stamp.source_name = name;
    state.tool = ToolKind::Stamp;
    let mut tool = crate::tools::stamp_place::StampPlaceTool::default();
    tool.press(state, xf, &pointer);
}

/// Anywhere else (or a miss): a picture file goes the way of File > Import Stamp Image (as before P10a), picture data
/// becomes the Stamp tool's picture; a sticker or PSD becomes a tray sticker and the Stamp tool's source.
fn elsewhere(state: &mut AppState, item: &Ready) {
    match item {
        Ready::Picture { path: Some(path), .. } => state.push(AppAction::ImportStampImage(Some(path.clone()))),
        Ready::Picture { name, bytes, path: None } => {
            let Some(blob) = add_blob(state, name, bytes) else { return };
            state.tools.stamp.source = StampToolSource::Blob(blob);
            state.tools.stamp.source_name = name.clone();
            state.push(AppAction::SetTool(ToolKind::Stamp));
            state.status_hint = "Click on the model to place the stamp".to_owned();
        }
        Ready::Psd { file, stem, stamp, psd } => {
            if let Some((id, toast)) = crate::link::flows::psd_sticker(state, psd, *stamp, file, stem, None) {
                crate::link::flows::psd_as_source(state, id, file, stem, toast);
            }
        }
        Ready::Sticker { .. } => {
            if let Some(id) = to_tray_sticker(state, item) {
                crate::sticker::tray::use_as_source(state, id);
            }
        }
    }
}

/// A drop on a stamp's row or on the Sticker tab's thumbnail.
fn on_stamp(state: &mut AppState, stamp: StampId, item: &Ready) {
    let Some(source) = state.doc().and_then(|d| d.stamp(stamp)).map(|s| s.source.clone()) else { return };
    match (&source, item) {
        (StampSource::Text(_), _) => {
            state.toasts.info("A text stamp's picture cannot be replaced: drop it on the model, or on an image or sticker stamp");
        }
        (StampSource::Image { .. }, Ready::Picture { name, bytes, .. }) => {
            let Some(blob) = add_blob(state, name, bytes) else { return };
            let r = state.doc_mut().map(|d| d.set_stamp_source(stamp, StampSource::Image { blob }, wte_core::Commit::Final));
            if let Some(r) = r {
                crate::dispatch::edited(state, r);
            }
        }
        (StampSource::Sticker { sticker }, Ready::Picture { .. }) => {
            // The picture becomes the top layer of this stamp's sticker; a sticker other stamps show too is first
            // detached, so the drop changes only the stamp it was dropped on.
            let shared = state.doc().is_some_and(|d| d.sticker_users(*sticker).len() > 1);
            if shared {
                let r = state.doc_mut().map(|d| d.detach_sticker(stamp));
                if r.and_then(|r| crate::dispatch::edited(state, r)).is_none() {
                    return;
                }
            }
            let Some(layer) = sticker_doc_of(state, item) else { return };
            let r = state.doc_mut().map(|d| {
                d.set_sticker(
                    wte_core::StickerRef::OfStamp(stamp, wte_core::ShareScope::ThisStamp),
                    move |s| {
                        let offset = (centre_of(s.artboard) - centre_of(layer.artboard)).round();
                        for mut added in layer.layers {
                            added.id = s.alloc_layer();
                            added.transform.translate += offset;
                            s.layers.push(added);
                        }
                    },
                    "Add Picture Layer",
                    wte_core::Commit::Final,
                )
            });
            if let Some(r) = r {
                crate::dispatch::edited(state, r);
            }
        }
        (_, Ready::Sticker { .. } | Ready::Psd { .. }) => {
            // The stamp shows the dropped sticker (an image stamp becomes a sticker stamp for it); other stamps that
            // showed the same picture keep theirs.
            let Some(id) = to_tray_sticker(state, item) else { return };
            if matches!(source, StampSource::Image { .. }) {
                let r = state.doc_mut().map(|d| d.promote_stamp(stamp, wte_core::ShareScope::ThisStamp));
                if r.and_then(|r| crate::dispatch::edited(state, r)).is_none() {
                    return;
                }
            }
            let r = state.doc_mut().map(|d| d.set_stamp_source(stamp, StampSource::Sticker { sticker: id }, wte_core::Commit::Final));
            if let Some(r) = r {
                crate::dispatch::edited(state, r);
            }
        }
    }
}

/// The centre of an artboard (layer space).
fn centre_of(a: wte_core::IRect) -> glam::Vec2 {
    glam::Vec2::new(a.x0 as f32 + a.width() as f32 * 0.5, a.y0 as f32 + a.height() as f32 * 0.5)
}

/// A picture as an image blob of the document (a warning toast when it cannot be decoded).
fn add_blob(state: &mut AppState, name: &str, bytes: &Arc<[u8]>) -> Option<wte_core::BlobId> {
    let doc = state.doc_mut()?;
    match doc.add_image_blob(name, Arc::clone(bytes)) {
        Ok(blob) => Some(blob),
        Err(e) => {
            state.toasts.warn(format!("Cannot use {name} as a stamp: {e}"));
            None
        }
    }
}

/// The item as a new tray sticker (kept: one entry "Add Sticker"); a picture as a one-layer sticker.
fn to_tray_sticker(state: &mut AppState, item: &Ready) -> Option<wte_core::StickerId> {
    match item {
        Ready::Picture { name, bytes, .. } => {
            let blob = add_blob(state, name, bytes)?;
            let stem = Path::new(name).file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
            crate::sticker::tray::new_from_blob(state, blob, &stem)
        }
        Ready::Sticker { file } => {
            let r = state.doc_mut()?.import_wtes(file);
            crate::dispatch::edited(state, r)
        }
        Ready::Psd { file, stem, stamp, psd } => {
            let (id, toast) = crate::link::flows::psd_sticker(state, psd, *stamp, file, stem, None)?;
            if let Some(t) = toast {
                state.toasts.info(t);
            }
            Some(id)
        }
    }
}

/// The item as a sticker document whose pictures are in the document's blob store (for its layers): a picture as a
/// one-layer sticker; a `.wtes` / `.psd` through a tray sticker that is removed again (both are entries of the drop's
/// one group).
fn sticker_doc_of(state: &mut AppState, item: &Ready) -> Option<StickerDoc> {
    if let Ready::Picture { name, bytes, .. } = item {
        let blob = add_blob(state, name, bytes)?;
        let size = state.doc_mut()?.image_size(blob).ok()?;
        let stem = Path::new(name).file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
        return Some(StickerDoc::from_picture(&stem, wte_core::PicturePixels::Blob(blob), size));
    }
    let id = to_tray_sticker(state, item)?;
    let doc = state.doc()?.sticker(id).map(|d| (**d).clone());
    if let Some(d) = state.doc_mut() {
        let _ = d.remove_sticker(id);
    }
    doc
}

/// What a drop of several files does (8.9): one project or model is opened (`.wtep` first, else the first
/// `.obj` / `.gltf` / `.glb` / `.fbx`); images, PSD and sticker files become stamp sources only when no project or
/// model is part of the drop (a model's texture dropped together with it is found through the model itself: its MTL
/// or glTF); every other file (`.mtl`, `.bin`, folders, unknown types) is ignored with one info toast instead of taking
/// the document's place.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct DropPlan {
    pub document: Option<std::path::PathBuf>,
    pub stamps: Vec<std::path::PathBuf>,
    pub ignored: Vec<std::path::PathBuf>,
    /// Why the `ignored` files are not opened (for the toast).
    pub reason: &'static str,
}

pub(crate) fn plan_drop(paths: &[std::path::PathBuf]) -> DropPlan {
    let is_project = |p: &std::path::PathBuf| extension_of(p) == wte_core::PROJECT_EXTENSION;
    let is_model = |p: &std::path::PathBuf| MODEL_EXTENSIONS.contains(&extension_of(p).as_str());
    let document = paths.iter().find(|p| is_project(p)).or_else(|| paths.iter().find(|p| is_model(p))).cloned();
    let mut plan = DropPlan { document, ..DropPlan::default() };
    for path in paths {
        if plan.document.as_ref() == Some(path) {
            continue;
        }
        if is_placeable_extension(&extension_of(path)) && plan.document.is_none() {
            plan.stamps.push(path.clone());
        } else {
            plan.ignored.push(path.clone());
        }
    }
    plan.reason = match &plan.document {
        Some(_) => "one project or model is opened per drop; a model finds its textures and material files itself",
        None => "not a model, project, image, PSD or sticker file",
    };
    plan
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Image extensions accepted as stamp sources (same list as the decoder: png, jpeg, tga, bmp, tiff, webp, dds).
pub const IMAGE_EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "webp", "dds"];
/// Model extensions of the import flow (file dialog filter, drag and drop, `OpenPath` / command line).
pub const MODEL_EXTENSIONS: [&str; 4] = ["obj", "gltf", "glb", "fbx"];

/// Lower-case extension of a path ("" without one).
pub fn extension_of(path: &std::path::Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Stickers P9a (STICKER_PLAN 5.8): a PSD / PSB file, imported as a sticker.
pub fn is_psd_path(path: &std::path::Path) -> bool {
    matches!(extension_of(path).as_str(), "psd" | "psb")
}

/// Stickers P10a: a sticker file (`.wtes`), added to the tray.
pub fn is_wtes_path(path: &std::path::Path) -> bool {
    extension_of(path) == wte_core::WTES_EXTENSION
}

/// True for the image extensions of `IMAGE_EXTENSIONS`.
pub fn is_image_path(path: &std::path::Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension_of(path).as_str())
}

/// A picture, PSD or sticker file extension (what a drop places at a point).
fn is_placeable_extension(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext) || matches!(ext, "psd" | "psb") || ext == wte_core::WTES_EXTENSION
}

#[cfg(test)]
mod tests;
