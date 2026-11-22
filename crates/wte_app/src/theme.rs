// Owner: app-shell (A1). Theme and icons of ARCHITECTURE.md 8.12: dark visuals, the procedurally generated window icon
// and the vector-painted tool icons (no image assets).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};

use crate::state::ToolKind;

/// Accent colour of the app (selection, focus).
pub const ACCENT: Color32 = Color32::from_rgb(70, 140, 220);

/// Applies the visuals of the app (dark, slightly tighter spacing).
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(6.0, 4.0);
        style.spacing.button_padding = vec2(5.0, 2.0);
        style.spacing.interact_size.y = 20.0;
        style.spacing.menu_margin = egui::Margin::same(4);
        style.spacing.window_margin = egui::Margin::same(8);
        style.visuals.selection.bg_fill = ACCENT.linear_multiply(0.8);
        style.visuals.hyperlink_color = ACCENT;
    });
    for file in FALLBACK_FONT_FILES {
        ctx.add_font_provider(Arc::new(SystemFallbackFont { file, data: OnceLock::new() }));
    }
}

/// Windows system fonts asked, in this order, for characters the built-in egui fonts lack. egui's own widgets use a few
/// symbols the built-in fonts do not have (the colour picker's copy button is "📋"), and eframe's `system_fonts` feature is
/// not enabled. egui checks the glyph itself and asks the next provider when a font lacks it.
const FALLBACK_FONT_FILES: [&str; 3] = ["seguisym.ttf", "seguiemj.ttf", "segoeui.ttf"];

/// One system fallback font, read from `%WINDIR%\Fonts` the first time egui asks for it (never when nothing is missing).
struct SystemFallbackFont {
    file: &'static str,
    data: OnceLock<Option<egui::FontData>>,
}

impl egui::FontProvider for SystemFallbackFont {
    fn font_for(&self, _request: &egui::FallbackRequest<'_>) -> Option<egui::FontInsert> {
        let data = self.data.get_or_init(|| {
            let dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
            let path = dir.join("Fonts").join(self.file);
            match std::fs::read(&path) {
                Ok(bytes) => Some(egui::FontData::from_owned(bytes)),
                Err(e) => {
                    log::info!("fallback font {} not available: {e}", path.display());
                    None
                }
            }
        });
        Some(egui::FontInsert::new(&format!("system:{}", self.file), data.clone()?, Vec::new()))
    }
}

/// Side of the window icon in pixels.
const ICON_SIDE: u32 = 64;

/// The window icon: 64x64 RGBA generated at runtime, a lit UV-checker sphere glyph.
pub fn app_icon() -> egui::IconData {
    let side = ICON_SIDE as usize;
    let mut rgba = vec![0u8; side * side * 4];
    let radius = side as f32 * 0.5 - 1.5;
    let center = side as f32 * 0.5;
    let light = glam::Vec3::new(-0.45, 0.55, 0.7).normalize();
    // 4x4 supersampling for a smooth rim.
    const SS: usize = 4;
    for y in 0..side {
        for x in 0..side {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - center;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - center;
                    let (nx, ny) = (px / radius, -py / radius);
                    let r2 = nx * nx + ny * ny;
                    if r2 > 1.0 {
                        continue;
                    }
                    let n = glam::Vec3::new(nx, ny, (1.0 - r2).sqrt());
                    // Rotate the sphere a little so the checker reads as wrapped around it.
                    let rot = glam::Quat::from_rotation_y(0.5) * glam::Quat::from_rotation_x(0.35);
                    let m = rot * n;
                    let u = 0.5 + m.x.atan2(m.z) / std::f32::consts::TAU;
                    let v = 0.5 - m.y.asin() / std::f32::consts::PI;
                    let cell = ((u * 8.0).floor() as i32 + (v * 4.0).floor() as i32).rem_euclid(2);
                    let base = if cell == 0 { [70.0, 140.0, 220.0] } else { [235.0, 240.0, 245.0] };
                    let shade = 0.35 + 0.65 * n.dot(light).max(0.0);
                    for c in 0..3 {
                        acc[c] += base[c] * shade;
                    }
                    acc[3] += 255.0;
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let o = (y * side + x) * 4;
            if a > 0.0 {
                // `acc` holds colour * coverage: un-premultiply for the straight-alpha icon.
                let cov = acc[3] / 255.0;
                for c in 0..3 {
                    rgba[o + c] = (acc[c] / cov).round().clamp(0.0, 255.0) as u8;
                }
                rgba[o + 3] = a.round() as u8;
            }
        }
    }
    egui::IconData { rgba, width: ICON_SIDE, height: ICON_SIDE }
}

/// Paints the vector icon of a tool into `rect`.
pub fn tool_icon(painter: &egui::Painter, rect: Rect, tool: ToolKind, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    // Icon coordinates are given in a 0..1 box centred in `rect`.
    let at = |x: f32, y: f32| -> Pos2 { pos2(c.x + (x - 0.5) * s, c.y + (y - 0.5) * s) };
    let w = (s * 0.075).max(1.0);
    let stroke = Stroke::new(w, color);
    match tool {
        ToolKind::Navigate => {
            // Orbit ring with two arrow heads.
            painter.circle_stroke(c, s * 0.3, stroke);
            painter.circle_filled(c, s * 0.07, color);
            arrow_head(painter, at(0.5, 0.2), vec2(1.0, 0.0), s * 0.13, color);
            arrow_head(painter, at(0.5, 0.8), vec2(-1.0, 0.0), s * 0.13, color);
        }
        ToolKind::Brush => {
            // Handle, ferrule and a rounded tip.
            painter.line_segment([at(0.82, 0.18), at(0.5, 0.5)], Stroke::new(w * 1.4, color));
            painter.line_segment([at(0.5, 0.5), at(0.42, 0.58)], Stroke::new(w * 2.6, color));
            let tip = vec![at(0.44, 0.52), at(0.48, 0.56), at(0.3, 0.8), at(0.18, 0.82), at(0.2, 0.7)];
            painter.add(egui::Shape::convex_polygon(tip, color, Stroke::NONE));
        }
        ToolKind::Eraser => {
            let body = vec![at(0.2, 0.62), at(0.55, 0.27), at(0.8, 0.52), at(0.45, 0.87)];
            painter.add(egui::Shape::closed_line(body, stroke));
            let felt = vec![at(0.2, 0.62), at(0.37, 0.45), at(0.62, 0.7), at(0.45, 0.87)];
            painter.add(egui::Shape::convex_polygon(felt, color, Stroke::NONE));
            painter.line_segment([at(0.15, 0.9), at(0.85, 0.9)], Stroke::new(w * 0.7, color));
        }
        ToolKind::Eyedropper => {
            painter.line_segment([at(0.2, 0.8), at(0.58, 0.42)], Stroke::new(w * 1.2, color));
            painter.line_segment([at(0.48, 0.36), at(0.64, 0.52)], Stroke::new(w * 1.6, color));
            painter.circle_filled(at(0.7, 0.3), s * 0.13, color);
            painter.circle_filled(at(0.18, 0.84), s * 0.045, color);
        }
        ToolKind::Fill => {
            // Tilted bucket with a drop.
            let bucket = vec![at(0.18, 0.45), at(0.48, 0.18), at(0.78, 0.48), at(0.48, 0.8)];
            painter.add(egui::Shape::closed_line(bucket, stroke));
            let paint = vec![at(0.18, 0.45), at(0.78, 0.48), at(0.48, 0.8)];
            painter.add(egui::Shape::convex_polygon(paint, color, Stroke::NONE));
            painter.line_segment([at(0.33, 0.31), at(0.18, 0.16)], stroke);
            painter.circle_filled(at(0.86, 0.72), s * 0.07, color);
        }
        ToolKind::Stamp => {
            // Knob, stem, block and the imprint line.
            painter.circle_filled(at(0.5, 0.2), s * 0.11, color);
            painter.line_segment([at(0.5, 0.3), at(0.5, 0.5)], Stroke::new(w * 1.6, color));
            painter.rect_filled(Rect::from_min_max(at(0.2, 0.5), at(0.8, 0.68)), s * 0.03, color);
            painter.line_segment([at(0.14, 0.82), at(0.86, 0.82)], stroke);
        }
        ToolKind::Text => {
            let bar = Stroke::new(w * 1.5, color);
            painter.line_segment([at(0.2, 0.2), at(0.8, 0.2)], bar);
            painter.line_segment([at(0.5, 0.2), at(0.5, 0.82)], bar);
            painter.line_segment([at(0.38, 0.82), at(0.62, 0.82)], stroke);
            painter.line_segment([at(0.2, 0.2), at(0.2, 0.3)], stroke);
            painter.line_segment([at(0.8, 0.2), at(0.8, 0.3)], stroke);
        }
        ToolKind::Transform => {
            // Dashed box with corner handles and a rotate knob above it.
            let r = Rect::from_min_max(at(0.22, 0.32), at(0.78, 0.84));
            painter.rect_stroke(r, 0.0, Stroke::new(w * 0.7, color), egui::StrokeKind::Middle);
            for corner in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
                painter.rect_filled(Rect::from_center_size(corner, vec2(s * 0.14, s * 0.14)), 0.0, color);
            }
            painter.line_segment([at(0.5, 0.32), at(0.5, 0.16)], Stroke::new(w * 0.7, color));
            painter.circle_stroke(at(0.5, 0.12), s * 0.06, stroke);
        }
    }
}

/// A filled triangular arrow head at `tip` pointing along `dir`.
fn arrow_head(painter: &egui::Painter, tip: Pos2, dir: egui::Vec2, size: f32, color: Color32) {
    let d = dir.normalized();
    let n = d.rot90();
    let back = tip - d * size;
    let points = vec![tip + d * size * 0.3, back + n * size * 0.6, back - n * size * 0.6];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Runs one headless egui pass in a unit test and discards its texture deltas (epaint panics on unapplied ones).
#[cfg(test)]
pub(crate) fn run_test_pass(ctx: &egui::Context, input: egui::RawInput, f: impl FnMut(&mut egui::Ui)) {
    ctx.run_ui(input, f).textures_delta.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_a_round_glyph_on_transparent_background() {
        let icon = app_icon();
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        let alpha = |x: usize, y: usize| icon.rgba[(y * 64 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "corners are transparent");
        assert_eq!(alpha(63, 63), 0);
        assert_eq!(alpha(32, 32), 255, "the centre is opaque");
        // Both checker colours appear.
        let mut blue = false;
        let mut white = false;
        for px in icon.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255) {
            blue |= i32::from(px[2]) > i32::from(px[0]) + 60;
            white |= px[0] > 150 && px[1] > 150 && px[2] > 150;
        }
        assert!(blue && white);
    }

    #[test]
    fn the_symbol_fallback_font_is_found_and_read_once() {
        use egui::FontProvider;
        let provider = SystemFallbackFont { file: FALLBACK_FONT_FILES[0], data: OnceLock::new() };
        let request = egui::FallbackRequest { cluster: "\u{1F4CB}", family: &egui::FontFamily::Proportional };
        let first = provider.font_for(&request).expect("Segoe UI Symbol is part of every Windows install");
        assert_eq!(first.name, "system:seguisym.ttf");
        assert!(first.data.bytes().len() > 100_000);
        let second = provider.font_for(&request).unwrap();
        assert!(Arc::ptr_eq(&first.data.font, &second.data.font), "the file is read only once");
        let missing = SystemFallbackFont { file: "no_such_font_file.ttf", data: OnceLock::new() };
        assert!(missing.font_for(&request).is_none());
    }

    #[test]
    fn every_tool_icon_paints_something() {
        let ctx = egui::Context::default();
        let tools = [
            ToolKind::Navigate,
            ToolKind::Brush,
            ToolKind::Eraser,
            ToolKind::Eyedropper,
            ToolKind::Fill,
            ToolKind::Stamp,
            ToolKind::Text,
            ToolKind::Transform,
        ];
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| {
            apply(ui.ctx());
            for tool in tools {
                let painter = ui.painter().clone();
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(24.0, 24.0));
                let count = || painter.ctx().graphics(|g| g.get(painter.layer_id()).map_or(0, |l| l.next_idx().0));
                let before = count();
                tool_icon(&painter, rect, tool, Color32::WHITE);
                let after = count();
                assert!(after > before, "{tool:?} painted nothing");
            }
        });
    }
}
