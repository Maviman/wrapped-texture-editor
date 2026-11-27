// Owner: stickers P8 (STICKER_PLAN 3.3 "Text", 3.2 "Text tab"; ARCHITECTURE 8.15). The sticker Text tool: a click on
// a text layer selects it; a click elsewhere adds a text layer there (the Text tool's settings of the Text tab, the
// primary colour, an em of a quarter of the artboard's shorter side) and focuses the content field (the Text block in
// the Tool tab). The layer's `TextSpec` is then edited by the Text block and the Text tab (`panels/text_block.rs`,
// `text_tab.rs`, `TextTarget::Layer`): every change is
// `StickerAction::SetLayerText` (Preview while typed or dragged, one Final: one entry "Edit Sticker Text").

use std::sync::Arc;

use glam::Vec2;
use wte_core::{StickerContent, StickerLayer, StickerLayerId, TextSpec};

use crate::state::AppState;
use crate::sticker::{edit_of, layered, set_active, shown};

/// History labels.
pub(crate) const ADD_TEXT_LABEL: &str = "Add Text Layer";
pub(crate) const EDIT_TEXT_LABEL: &str = "Edit Sticker Text";

/// The em of a new text layer on an artboard of `size` (layer-space pixels): a quarter of the shorter side, 12..=256.
pub(crate) fn default_em(size: [u32; 2]) -> f32 {
    (size[0].min(size[1]) as f32 / 4.0).clamp(12.0, 256.0)
}

/// A layer name from a text: its first line, at most 24 characters ("Text" when empty).
pub(crate) fn name_of(content: &str) -> String {
    let line: String = content.lines().next().unwrap_or("").trim().chars().take(24).collect();
    if line.is_empty() { "Text".to_owned() } else { line }
}

/// The Text tool's click at the layer-space point `p`: selects the text layer under it, or adds a new text layer
/// there.
pub(crate) fn click(state: &mut AppState, p: Vec2) {
    let Some(edit) = edit_of(state).cloned() else { return };
    // A text layer under the pointer becomes the active layer.
    let hit = match state.session.as_mut().and_then(|s| shown(&mut s.doc, &edit)) {
        Some(crate::sticker::Shown::Sticker { id, .. }) => state.doc_mut().and_then(|d| d.sticker_layer_at(id, p)),
        _ => None,
    };
    if let Some(id) = hit
        && layered(state, Some(id)).is_some_and(|(d, _)| d.layer(id).is_some_and(|l| matches!(l.content, StickerContent::Text(_))))
    {
        set_active(state, id);
        state.sticker_ui.focus_text = true;
        return;
    }
    state.push(crate::action::AppAction::Sticker(crate::sticker::StickerAction::AddTextLayer(Some(p.to_array()))));
}

/// The spec of a new text layer: the Text tool's settings with a font of the document, the primary colour.
fn new_spec(state: &mut AppState, size: [u32; 2]) -> Option<TextSpec> {
    let draft = crate::panels::text_block::draft_spec(&state.tools.text);
    let font = crate::panels::text_block::resolve_font(state, &draft.font.family, &draft.font.style)?;
    let mut spec = draft;
    spec.font = font;
    spec.color = state.colors.primary;
    spec.size_px = default_em(size);
    if spec.content.trim().is_empty() {
        spec.content = "Text".to_owned();
    }
    Some(spec)
}

/// "Add text": a text layer with its anchor at `at` (layer space; `None` = the artboard centre) above the active layer,
/// which becomes the active layer; the content field gets the focus. One entry.
pub(crate) fn add(state: &mut AppState, at: Option<Vec2>) {
    let Some((doc, _)) = layered(state, None).or_else(|| {
        let e = edit_of(state)?.clone();
        let session = state.session.as_mut()?;
        let d = shown(&mut session.doc, &e)?.layers()?.clone();
        let id = d.layers.first()?.id;
        Some((d, id))
    }) else {
        state.status_hint = "Convert the text to a layered sticker first".to_owned();
        return;
    };
    let size = doc.size();
    let Some(spec) = new_spec(state, size) else { return };
    let centre = Vec2::new(doc.artboard.x0 as f32 + size[0] as f32 * 0.5, doc.artboard.y0 as f32 + size[1] as f32 * 0.5);
    let at = at.filter(|p| p.is_finite()).unwrap_or(centre).round();
    let new_id = StickerLayerId(doc.next_layer);
    let above = edit_of(state).and_then(|e| e.active_layer).and_then(|a| doc.layers.iter().position(|l| l.id == a));
    let name = name_of(&spec.content);
    let done = crate::sticker::apply_edit(state, ADD_TEXT_LABEL, wte_core::Commit::Final, move |d| {
        let id = d.alloc_layer();
        let mut layer = StickerLayer::new(id, &name, StickerContent::Text(Box::new(spec)));
        layer.transform.translate = at;
        let i = above.map_or(d.layers.len(), |i| (i + 1).min(d.layers.len()));
        d.layers.insert(i, layer);
    });
    if done {
        set_active(state, new_id);
        state.sticker_ui.focus_text = true;
    }
}

/// The spec of text layer `id` of the open editor.
pub(crate) fn spec_of(state: &AppState, id: StickerLayerId) -> Option<TextSpec> {
    let session = state.session.as_ref()?;
    let edit = session.sticker_edit.as_ref()?;
    let doc: Arc<wte_core::StickerDoc> = match edit.target {
        crate::state::StickerTarget::Stamp(stamp) => match session.doc.stamp(stamp)?.source {
            wte_core::StampSource::Sticker { sticker } => Arc::clone(session.doc.sticker(sticker)?),
            _ => return None,
        },
        crate::state::StickerTarget::Tray(id) => Arc::clone(session.doc.sticker(id)?),
    };
    match &doc.layer(id)?.content {
        StickerContent::Text(spec) => Some((**spec).clone()),
        _ => None,
    }
}

/// The active layer of the open editor when it is a text layer (the Text tab's `TextTarget::Layer`).
pub(crate) fn active_text_layer(state: &AppState) -> Option<StickerLayerId> {
    if state.view_mode != crate::state::ViewMode::Sticker {
        return None;
    }
    let id = edit_of(state)?.active_layer?;
    spec_of(state, id).map(|_| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_text_layer_gets_a_readable_em_and_a_short_name() {
        assert_eq!(default_em([512, 512]), 128.0);
        assert_eq!(default_em([20, 4000]), 12.0);
        assert_eq!(default_em([4096, 4096]), 256.0);
        assert_eq!(name_of("  Hello world, this is a long first line\nsecond"), "Hello world, this is a l");
        assert_eq!(name_of("\n"), "Text");
    }
}
