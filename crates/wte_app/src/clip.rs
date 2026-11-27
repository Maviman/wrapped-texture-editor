// Owner: stickers P10b (STICKER_PLAN 6.3, 13.5; ARCHITECTURE 8.1, 8.3, 8.17). Copy, Cut and Paste with the clipboard
// (`AppState::clipboard`: the Windows clipboard in an interactive run, an in-memory one in self-test runs and tests).
//
// How the keys arrive (verified in egui-winit 0.37, `lib.rs` 1129-1151, eframe without its clipboard feature):
// Ctrl+C / Ctrl+Insert become `Event::Copy` and Ctrl+X / Shift+Delete `Event::Cut`, never a key event; Ctrl+V /
// Shift+Insert become `Event::Paste(in-app string)` only when egui-winit's own in-app string is not empty, else
// nothing at all. So (6.3): `input_hook` (from `raw_input_hook`) takes `Event::Copy` / `Event::Cut` when no text field
// has the keyboard and queues `ClipAction::Copy` / `Cut`; the paste chord comes from WTE's key subclass
// (`ole::subclass`, `OleEvent::Chord(Paste)`): every `Event::Paste` egui-winit made is removed, and either a text field
// gets the clipboard's real text (`Event::Paste(text)`), or `ClipAction::Paste` is queued. Text copied in an egui field
// (`OutputCommand::CopyText`) is written to the clipboard at the end of `App::ui` (`after_ui`).
//
// What is copied (6.3): a selected stamp's finished picture with its styles ("what you see") as PNG + CF_DIBV5, plus
// WTE.Sticker.v1 (a `.wtes` with the stamp styles) for a sticker stamp and for an image stamp with styles; else the
// active paint layer at texture size; in the Sticker view the active picture layer's pixels (its mask applied, no layer
// styles), cut to the selection when there is one. The pixels are taken on the UI thread (shared buffers, no copy of a
// texture) and encoded on a worker; the clipboard is written when the worker is done (`poll`), and a Paste first waits
// for a copy still being encoded (`finish_copies`). Paste goes through the drop pipeline of `dnd.rs` with a spot:
// outside the Sticker view the picture becomes the Stamp tool's source (nothing is placed until a click), Paste at
// View Centre places it at once in the middle of the view (a miss is a plain Paste), in the Sticker view it becomes a
// new layer centred in the visible canvas.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use eframe::egui;
use wte_core::sticker::select;
use wte_core::{AlphaBuf, IRect, LayerKind, PixelBuf, StampId, StampSource, StickerContent, StickerDoc};

use crate::action::AppAction;
use crate::app::WteApp;
use crate::dnd::Spot;
use crate::ole::Chord;
use crate::ole::clipboard::{self, ClipFormat, Clipboard};
use crate::state::{AppState, Dialog, ViewId, ViewMode};
use crate::sticker::StickerAction;

/// A clipboard command (Edit menu, the chords, the self-test).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClipAction {
    Copy,
    /// Copy, then the selected pixels go (the Sticker view only).
    Cut,
    Paste,
    /// Paste and place at once in the middle of the view (a miss: a plain Paste).
    PasteAtViewCentre,
    /// Help > Clipboard contents...: the formats on the clipboard (13.5).
    ShowContents,
}

/// Status line of Cut outside the Sticker view.
pub(crate) const CUT_HINT: &str = "Cut takes selected pixels in the Sticker view; use Copy here";
/// Status line of Cut without a selection.
pub(crate) const CUT_SELECT_HINT: &str = "Select an area first: Cut takes the selected pixels";
/// Status line of Copy with nothing to copy.
pub(crate) const NOTHING_TO_COPY: &str = "Select a stamp, or a paint layer with pixels, to copy";
/// Status line of Copy on a text or shape layer of the Sticker view.
pub(crate) const COPY_PICTURE_HINT: &str = "Copy takes a picture layer's pixels: rasterize this layer first";

/// What a copy worker hands back: what was copied (for the status line) and the clipboard entries.
type CopyDone = Result<(String, Vec<(ClipFormat, Vec<u8>)>), String>;

/// The copy being encoded (`AppState::clip_ui`). Only the latest Copy counts (check P10b): a new copy supersedes
/// one still being encoded - its result is never written (a slow 4096 x 4096 layer finishing after a quick stamp copy
/// would otherwise put the older picture on the clipboard), and its worker stops before the next encoding step (a held
/// Ctrl+C repeats `Event::Copy`; without this every repeat encoded the whole picture at the same time).
#[derive(Default)]
pub struct ClipUi {
    job: Option<CopyJob>,
    /// Copies written to the clipboard so far (the self-test's and the tests' counter).
    pub(crate) copies: u64,
}

/// One copy on its worker: its result, and the flag that tells the worker it was superseded.
struct CopyJob {
    rx: mpsc::Receiver<CopyDone>,
    cancel: Arc<AtomicBool>,
}

impl ClipUi {
    /// True while a copy is being encoded (the self-test's idle check).
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }
}

/// True when a text field (or a selected label text) has the keyboard: the chords are then egui's.
fn text_has_keyboard(ctx: &egui::Context) -> bool {
    ctx.text_edit_focused()
        || ctx.with_plugin::<egui::text_selection::LabelSelectionState, _>(|s| s.has_selection()).unwrap_or(false)
}

/// `raw_input_hook` (8.3): the chords queued by the key subclass (or the self-test) and egui-winit's Copy / Cut /
/// Paste events of this frame (`route`).
pub fn input_hook(ctx: &egui::Context, raw: &mut egui::RawInput, state: &mut AppState) {
    let chords = crate::ole::take_chords();
    if chords.is_empty() && !raw.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)) {
        return;
    }
    let blocked = !matches!(state.dialog, Dialog::None) || state.busy.as_ref().is_some_and(|b| b.modal);
    for action in route(&mut raw.events, &chords, text_has_keyboard(ctx), blocked, &*state.clipboard) {
        state.push(AppAction::Clip(action));
    }
}

/// The routing of 6.3 on one frame's events (pure apart from reading the clipboard's text):
/// (a) `Event::Copy` / `Event::Cut` without a text field: removed, `Copy` / `Cut` queued; with one: left for egui.
/// (b) a paste chord with a text field: every `Event::Paste` egui-winit made from its in-app string goes, and exactly
///     one `Event::Paste(clipboard text)` comes in its place (`\r\n` as `\n`);
/// (c) a paste chord without a text field: those events go and `Paste` is queued;
/// (d) a paste chord with a text field but no text on the clipboard (or it cannot be read): egui-winit's events stay.
/// `blocked` (a dialog or a modal job): nothing is queued (text fields still get their copy and paste).
pub(crate) fn route(
    events: &mut Vec<egui::Event>,
    chords: &[Chord],
    text_focus: bool,
    blocked: bool,
    cb: &dyn Clipboard,
) -> Vec<ClipAction> {
    let mut out = Vec::new();
    if !text_focus {
        events.retain(|e| match e {
            egui::Event::Copy => {
                out.push(ClipAction::Copy);
                false
            }
            egui::Event::Cut => {
                out.push(ClipAction::Cut);
                false
            }
            _ => true,
        });
    }
    // The self-test's Copy / Cut chords are the events egui-winit makes; only Paste comes as a chord from the subclass.
    if chords.contains(&Chord::Paste) {
        let in_app: Vec<egui::Event> = events.iter().filter(|e| matches!(e, egui::Event::Paste(_))).cloned().collect();
        events.retain(|e| !matches!(e, egui::Event::Paste(_)));
        if text_focus {
            match clipboard::read_text(cb) {
                Ok(Some(text)) if !text.is_empty() => events.push(egui::Event::Paste(text)),
                Ok(_) => events.extend(in_app),
                Err(e) => {
                    log::warn!("paste: the clipboard's text could not be read ({e}); egui's own text is pasted");
                    events.extend(in_app);
                }
            }
        } else {
            out.push(ClipAction::Paste);
        }
    }
    if blocked {
        out.clear();
    }
    out
}

/// The end of `App::ui` (6.3 item 4): text an egui field copied this frame (`OutputCommand::CopyText`) is written to
/// the clipboard too; the command stays (egui-winit keeps its in-app copy).
pub fn after_ui(ctx: &egui::Context, state: &mut AppState) {
    let texts: Vec<String> = ctx.output(|o| {
        o.commands
            .iter()
            .filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None })
            .collect()
    });
    let Some(text) = texts.into_iter().rev().find(|t| !t.is_empty()) else { return };
    if let Err(e) = clipboard::write_text(&mut *state.clipboard, &text) {
        state.toasts.warn(format!("The copied text could not be put on the clipboard: {e}"));
    }
}

/// Writes the copy when its encoding is done (every frame, `App::ui`).
pub fn poll(state: &mut AppState) {
    let Some(job) = &state.clip_ui.job else { return };
    match job.rx.try_recv() {
        Ok(d) => {
            state.clip_ui.job = None;
            write_copy(state, d);
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => state.clip_ui.job = None,
    }
}

/// Waits for the copy still being encoded and writes it (before a Paste reads the clipboard).
pub(crate) fn finish_copies(state: &mut AppState) {
    if let Some(job) = state.clip_ui.job.take()
        && let Ok(d) = job.rx.recv()
    {
        write_copy(state, d);
    }
}

fn write_copy(state: &mut AppState, done: CopyDone) {
    match done.and_then(|(what, entries)| {
        let names: Vec<&str> = entries.iter().map(|(f, _)| f.name()).collect();
        log::info!("copy: {what} as {}", names.join(", "));
        state.clipboard.write(entries).map(|()| what)
    }) {
        Ok(what) => {
            state.clip_ui.copies += 1;
            state.status_hint = format!("Copied {what}");
        }
        Err(e) => state.toasts.warn(format!("Nothing was copied: {e}")),
    }
}

/// Runs a clipboard command (`dispatch`).
pub(crate) fn run(app: &mut WteApp, action: ClipAction) {
    let ctx = app.ctx.clone();
    run_with(&ctx, &mut app.state, action);
}

/// `run` on the state alone (the tests).
pub(crate) fn run_with(ctx: &egui::Context, state: &mut AppState, action: ClipAction) {
    match action {
        ClipAction::Copy => copy(ctx, state, false),
        ClipAction::Cut => copy(ctx, state, true),
        ClipAction::Paste => paste(ctx, state, false),
        ClipAction::PasteAtViewCentre => paste(ctx, state, true),
        ClipAction::ShowContents => show_contents(state),
    }
}

/// A copy on a worker: `take` makes the picture (and the `.wtes`) from what the UI thread took; the PNG and the DIB
/// are encoded there too.
/// Stickers P11: drag out (`dragout.rs`) takes its pictures the same way.
pub(crate) type Take = Box<dyn FnOnce() -> Result<(Arc<PixelBuf>, Option<Vec<u8>>), String> + Send>;

/// `sized`: the status words get the picture's size (" (W x H pixels)"), known only once the worker has made it.
fn start_copy(ctx: &egui::Context, state: &mut AppState, what: String, sized: bool, take: Take) {
    // The copy being encoded is superseded: its result is dropped with its receiver, and its worker stops early.
    if let Some(old) = state.clip_ui.job.take() {
        old.cancel.store(true, Ordering::Release);
    }
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let superseded = || stop.load(Ordering::Acquire);
        if superseded() {
            return;
        }
        let done = take().and_then(|(pixels, wtes)| {
            if superseded() {
                return Err(SUPERSEDED.to_owned());
            }
            let png = wte_core::encode_png(pixels.width, pixels.height, &pixels.data, wte_core::PngLevel::Fast).map_err(|e| e.to_string())?;
            if superseded() {
                return Err(SUPERSEDED.to_owned());
            }
            let what = if sized { format!("{what} ({} x {} pixels)", pixels.width, pixels.height) } else { what };
            Ok((what, clipboard::picture_entries(png, &pixels, wtes.as_deref())))
        });
        if !superseded() {
            let _ = tx.send(done);
            ctx.request_repaint();
        }
    });
    state.clip_ui.job = Some(CopyJob { rx, cancel });
}

/// What a superseded copy's worker ends with (never shown: its receiver is gone).
const SUPERSEDED: &str = "a later copy replaced this one";

fn copy(ctx: &egui::Context, state: &mut AppState, cut: bool) {
    if state.session.is_none() {
        state.status_hint = "Open a model first".to_owned();
        return;
    }
    if state.view_mode == ViewMode::Sticker && crate::sticker::edit_of(state).is_some() {
        copy_sticker_layer(ctx, state, cut);
        return;
    }
    if cut {
        state.status_hint = CUT_HINT.to_owned();
        return;
    }
    let session = state.session.as_ref().expect("checked");
    if let Some(stamp) = session.selected_stamp.filter(|id| session.doc.stamp(*id).is_some()) {
        copy_stamp(ctx, state, stamp);
        return;
    }
    let layer = session.doc.set(session.active_set).and_then(|set| {
        let layer = set.active_layer()?;
        Some((layer.name.clone(), layer.kind.clone(), layer.stamp_id(), [set.width, set.height]))
    });
    match layer {
        Some((_, _, Some(stamp), _)) => copy_stamp(ctx, state, stamp),
        Some((name, LayerKind::Raster(buf), None, [w, h])) if buf.width == w && buf.height == h && w > 0 && h > 0 => {
            start_copy(ctx, state, format!("the layer \"{name}\""), true, Box::new(move || Ok((buf, None))));
        }
        _ => state.status_hint = NOTHING_TO_COPY.to_owned(),
    }
}

/// A stamp's finished picture with its styles, and (a sticker stamp, an image stamp with styles) its `.wtes` with the
/// stamp styles.
fn copy_stamp(ctx: &egui::Context, state: &mut AppState, stamp: StampId) {
    match stamp_take(state, stamp) {
        Ok(Some(picked)) => start_copy(ctx, state, picked.what, true, picked.take),
        Ok(None) => {}
        Err(e) => state.toasts.warn(format!("Nothing was copied: {e}")),
    }
}

/// What Copy (and drag out) takes of something: the status words, the name a file of it gets, and the worker part.
pub(crate) struct Picked {
    pub what: String,
    pub name: String,
    pub take: Take,
}

/// A stamp's finished picture with its styles ("what you see": `stamp_picture(id, true)`) and, for a sticker stamp or an
/// image stamp with styles, its `.wtes` with the stamp styles (Copy, stickers P11 drag out). The base picture is taken
/// here; its tint and styles are drawn by the worker part (P11 check: drawing the styles of a large picture cost every
/// Layers-row drag's start up to a few hundred milliseconds on the UI thread). `Ok(None)` without a document or stamp.
pub(crate) fn stamp_take(state: &mut AppState, stamp: StampId) -> Result<Option<Picked>, String> {
    let Some(doc) = state.doc_mut() else { return Ok(None) };
    let Some(object) = doc.stamp(stamp).cloned() else { return Ok(None) };
    let parts = doc.stamp_picture_parts(stamp, true).map_err(|e| e.to_string())?;
    let styles = (!object.styles.is_default()).then(|| object.styles.clone());
    let sticker: Option<StickerDoc> = match &object.source {
        StampSource::Sticker { sticker } => doc.sticker(*sticker).map(|d| (**d).clone()),
        StampSource::Image { blob } if styles.is_some() => doc
            .image_size(*blob)
            .ok()
            .map(|size| StickerDoc::from_picture(&object.name, wte_core::PicturePixels::Blob(*blob), size)),
        _ => None,
    };
    let blobs = sticker.as_ref().map(|_| doc.blobs.clone());
    let what = format!("the stamp \"{}\"", object.name);
    Ok(Some(Picked {
        what,
        name: object.name.clone(),
        take: Box::new(move || {
            let picture = parts.finish();
            let wtes = match (sticker, blobs) {
                (Some(s), Some(b)) => match wte_core::write_wtes(&s, styles.as_ref(), &b, Some(&picture)) {
                    Ok(bytes) => Some(bytes),
                    Err(e) => {
                        log::warn!("copy: the sticker could not be written as WTE.Sticker.v1 ({e}); the picture is copied");
                        None
                    }
                },
                _ => None,
            };
            Ok((Arc::new(picture), wtes))
        }),
    }))
}

/// The Sticker view: the active picture layer's pixels (mask applied, no layer styles), cut to the selection.
fn copy_sticker_layer(ctx: &egui::Context, state: &mut AppState, cut: bool) {
    let Some(edit) = crate::sticker::edit_of(state).cloned() else { return };
    let Some(on) = crate::sticker::edit_ref(&edit) else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(shown) = crate::sticker::shown(&mut session.doc, &edit) else { return };
    let Some(doc) = shown.layers() else {
        state.status_hint = crate::sticker::tools::TEXT_STAMP_HINT.to_owned();
        return;
    };
    let Some(layer) = edit.active_layer.and_then(|id| doc.layer(id)).cloned() else {
        state.status_hint = "Pick a layer in the Layers tab first".to_owned();
        return;
    };
    let artboard = doc.artboard;
    if !matches!(layer.content, StickerContent::Picture(_)) {
        state.status_hint = COPY_PICTURE_HINT.to_owned();
        return;
    }
    let selection = edit.selection.as_ref().map(|s| s.mask.clone());
    if cut && selection.is_none() {
        state.status_hint = CUT_SELECT_HINT.to_owned();
        return;
    }
    let pixels = match session.doc.sticker_layer_pixels(on, layer.id) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return;
        }
    };
    let mask = layer.mask.as_ref().map(|m| Arc::clone(&m.pixels));
    let transform = layer.transform;
    let what = format!("the {} of the layer \"{}\"", if selection.is_some() { "selected pixels" } else { "pixels" }, layer.name);
    start_copy(ctx, state, what, false, Box::new(move || layer_pixels(&pixels, mask.as_deref(), selection.as_ref(), artboard, &transform).map(|p| (Arc::new(p), None))));
    if cut {
        state.push(AppAction::Sticker(StickerAction::DeleteSelected));
    }
}

/// The pixels a Sticker-view copy takes: the layer's picture with its mask applied, then (a selection on the
/// artboard) only the selected pixels, cut to their bounds. `Err` when the selection holds none of them.
pub(crate) fn layer_pixels(
    pixels: &PixelBuf,
    mask: Option<&AlphaBuf>,
    selection: Option<&AlphaBuf>,
    artboard: IRect,
    transform: &wte_core::LayerTransform,
) -> Result<PixelBuf, String> {
    let mut out = pixels.clone();
    if let Some(m) = mask.filter(|m| m.width == out.width && m.height == out.height) {
        for (px, &a) in out.data.as_chunks_mut::<4>().0.iter_mut().zip(&m.data) {
            px[3] = ((u32::from(px[3]) * u32::from(a) + 127) / 255) as u8;
        }
    }
    let Some(sel) = selection else { return Ok(out) };
    let seen = select::selection_in_content(sel, artboard, transform, [out.width, out.height]);
    let picked = select::extract(&out, &seen);
    let bounds = select::mask_bounds(&AlphaBuf::from_alpha(&picked));
    if bounds.is_empty() {
        return Err(crate::sticker::tools::move_selection::NOTHING_SELECTED_HINT.to_owned());
    }
    Ok(crop(&picked, bounds))
}

fn crop(p: &PixelBuf, r: IRect) -> PixelBuf {
    let (w, h) = (r.width(), r.height());
    let mut out = PixelBuf::new(w, h);
    for y in 0..h as usize {
        let s = ((r.y0 as usize + y) * p.width as usize + r.x0 as usize) * 4;
        out.data[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(&p.data[s..s + w as usize * 4]);
    }
    out
}

/// The label of a paste's one undo step (check P10b: it was "Drop").
pub(crate) const PASTE_LABEL: &str = "Paste";

/// The sentence of a paste that finds nothing usable (13.5).
pub(crate) fn nothing_text(names: &[String]) -> String {
    let offers = if names.is_empty() { "nothing".to_owned() } else { names.join(", ") };
    format!("Nothing WTE can paste. The clipboard offers: {offers}.")
}

/// Where a paste lands (6.3): the visible canvas's centre in the Sticker view; with `at_centre` the middle of the 3D
/// view (Split: its 3D half) or of the 2D view; else nowhere in particular (the Stamp tool's source).
pub(crate) fn paste_spot(state: &AppState, at_centre: bool) -> Spot {
    if state.view_mode == ViewMode::Sticker {
        return match state.sticker_canvas_rect {
            Some(r) if crate::sticker::edit_of(state).is_some() => Spot::Canvas(r.center()),
            _ => Spot::Elsewhere,
        };
    }
    if !at_centre {
        return Spot::Elsewhere;
    }
    let view = match state.view_mode {
        ViewMode::View3D | ViewMode::Split => ViewId::Main3D,
        ViewMode::View2D => ViewId::Main2D,
        _ => return Spot::Elsewhere,
    };
    state.view_rects.get(&view).map_or(Spot::Elsewhere, |r| Spot::View(view, r.center()))
}

fn paste(ctx: &egui::Context, state: &mut AppState, at_centre: bool) {
    finish_copies(state);
    if state.busy.as_ref().is_some_and(|b| b.modal) {
        state.toasts.warn("Please wait for the running operation to finish");
        return;
    }
    let names = state.clipboard.format_names().unwrap_or_else(|e| {
        log::warn!("paste: the clipboard's formats could not be read ({e})");
        Vec::new()
    });
    log::info!("paste: the clipboard offers {}", if names.is_empty() { "nothing".to_owned() } else { names.join(", ") });
    match clipboard::paste_item(&*state.clipboard) {
        Err(e) => state.toasts.warn(format!("Could not read the clipboard: {e}")),
        Ok(None) => state.toasts.info(nothing_text(&names)),
        Ok(Some((format, item))) => {
            log::info!("paste: {} taken", format.name());
            if state.session.is_none() {
                state.toasts.warn("Open a model first");
                return;
            }
            let spot = paste_spot(state, at_centre);
            crate::dnd::start_items(ctx, state, spot, vec![item], PASTE_LABEL);
        }
    }
}

/// Help > Clipboard contents... (13.5): what the clipboard offers, in a plain dialog.
fn show_contents(state: &mut AppState) {
    let lines = match state.clipboard.format_names() {
        Ok(names) if names.is_empty() => vec!["The clipboard is empty.".to_owned()],
        Ok(names) => {
            let mut lines = vec!["The clipboard offers these formats:".to_owned()];
            lines.extend(names.into_iter().map(|n| format!("    {n}")));
            lines.push(String::new());
            lines.push("WTE pastes WTE.Sticker.v1, PNG, CF_DIBV5, CF_DIB and picture files (CF_HDROP), in this order.".to_owned());
            lines
        }
        Err(e) => vec![format!("The clipboard could not be read: {e}")],
    };
    state.push(AppAction::ShowDialog(Dialog::Message { title: "Clipboard contents".to_owned(), lines }));
}

#[cfg(test)]
mod tests;
