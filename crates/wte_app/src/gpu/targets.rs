//! Offscreen view targets (ARCHITECTURE.md 8.4, D16): one per view, registered with egui as a native texture.

use eframe::{egui, wgpu};

use crate::gpu::Gpu;
use crate::state::ViewId;

/// Allocation quantum of view targets in pixels.
const QUANTUM: u32 = 64;

/// The allocated side for a used side: rounded up to a multiple of 64, clamped to `64..=max_side`.
fn alloc_side(used: u32, max_side: u32) -> u32 {
    let max_side = max_side.max(QUANTUM);
    used.max(1).div_ceil(QUANTUM).saturating_mul(QUANTUM).clamp(QUANTUM, max_side)
}

/// The used side for a requested side: `1..=max_side`.
fn used_side(px: u32, max_side: u32) -> u32 {
    px.clamp(1, max_side.max(1))
}

/// The GPU textures of one allocation.
struct TargetTextures {
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The multisampled colour attachment; `None` when `Gpu::msaa` is 1 (the pass renders into `resolve` directly).
    msaa: Option<(wgpu::Texture, wgpu::TextureView)>,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl TargetTextures {
    fn new(gpu: &Gpu, label: &str, size: [u32; 2]) -> TargetTextures {
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |suffix: &str, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("{label}_{suffix}")),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let resolve = make(
            "resolve",
            Gpu::COLOR_FORMAT,
            1,
            // COPY_SRC only in tests (GPU read-back).
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | if cfg!(test) { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() },
        );
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa = (gpu.msaa > 1).then(|| {
            let texture = make("msaa", Gpu::COLOR_FORMAT, gpu.msaa, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let msaa_view = view(&texture);
            (texture, msaa_view)
        });
        let depth = make("depth", Gpu::DEPTH_FORMAT, gpu.msaa, wgpu::TextureUsages::RENDER_ATTACHMENT);
        TargetTextures { resolve_view: view(&resolve), depth_view: view(&depth), resolve, msaa, depth }
    }

    fn destroy(&self) {
        self.resolve.destroy();
        if let Some((texture, _)) = &self.msaa {
            texture.destroy();
        }
        self.depth.destroy();
    }
}

/// Offscreen target of one view: resolve texture (1 sample), MSAA colour and depth, registered with egui.
pub struct ViewTarget {
    pub egui_id: egui::TextureId,
    label: String,
    textures: TargetTextures,
    alloc: [u32; 2],
    used: [u32; 2],
}

impl ViewTarget {
    /// 64x64 target, registered with `register_native_texture(.., FilterMode::Linear)`.
    pub fn new(gpu: &Gpu, label: &str) -> ViewTarget {
        let size = [QUANTUM, QUANTUM];
        let textures = TargetTextures::new(gpu, label, size);
        let egui_id =
            gpu.egui_renderer.write().register_native_texture(&gpu.device, &textures.resolve_view, wgpu::FilterMode::Linear);
        ViewTarget { egui_id, label: label.to_owned(), textures, alloc: size, used: size }
    }

    /// Allocation is rounded up to a multiple of 64 and clamped to `64..=max_side`; on reallocation the egui texture
    /// id is kept (`update_egui_texture_from_wgpu_texture`). The 64-px quantum avoids reallocating on every frame of a
    /// panel drag.
    pub fn ensure_size(&mut self, gpu: &Gpu, px: [u32; 2]) {
        let used = [used_side(px[0], gpu.max_side), used_side(px[1], gpu.max_side)];
        let alloc = [alloc_side(used[0], gpu.max_side), alloc_side(used[1], gpu.max_side)];
        self.used = used;
        if alloc == self.alloc {
            return;
        }
        let textures = TargetTextures::new(gpu, &self.label, alloc);
        gpu.egui_renderer.write().update_egui_texture_from_wgpu_texture(
            &gpu.device,
            &textures.resolve_view,
            wgpu::FilterMode::Linear,
            self.egui_id,
        );
        std::mem::replace(&mut self.textures, textures).destroy();
        self.alloc = alloc;
    }

    /// The used size in physical pixels.
    pub fn size_px(&self) -> [u32; 2] {
        self.used
    }

    /// `[0, 0]..used / alloc`, for `painter.image`.
    pub fn uv_rect(&self) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(self.used[0] as f32 / self.alloc[0] as f32, self.used[1] as f32 / self.alloc[1] as f32),
        )
    }

    /// Frees the egui texture (`free_texture`) and the GPU textures.
    #[allow(dead_code, reason = "8.4 contract API; the four view targets live as long as the app and are never freed early")]
    pub fn destroy(self, gpu: &Gpu) {
        gpu.egui_renderer.write().free_texture(&self.egui_id);
        self.textures.destroy();
    }

    /// Single-sample texture the MSAA colour resolves into (what egui shows).
    pub(crate) fn resolve_view(&self) -> &wgpu::TextureView {
        &self.textures.resolve_view
    }

    /// The multisampled colour attachment (`None` without MSAA: the pass renders into `resolve_view`).
    pub(crate) fn msaa_view(&self) -> Option<&wgpu::TextureView> {
        self.textures.msaa.as_ref().map(|(_, view)| view)
    }

    /// Depth attachment (`Gpu::msaa` samples).
    pub(crate) fn depth_view(&self) -> &wgpu::TextureView {
        &self.textures.depth_view
    }

    /// The resolve texture itself (GPU read-back in tests).
    #[cfg(test)]
    pub(crate) fn resolve_texture(&self) -> &wgpu::Texture {
        &self.textures.resolve
    }
}

/// The targets of all views.
pub struct ViewTargets {
    pub main3d: ViewTarget,
    pub main2d: ViewTarget,
    pub preview: ViewTarget,
    pub export_preview: ViewTarget,
}

impl ViewTargets {
    /// One 64x64 target per view.
    pub fn new(gpu: &Gpu) -> ViewTargets {
        ViewTargets {
            main3d: ViewTarget::new(gpu, "main3d"),
            main2d: ViewTarget::new(gpu, "main2d"),
            preview: ViewTarget::new(gpu, "preview"),
            export_preview: ViewTarget::new(gpu, "export_preview"),
        }
    }

    /// The target of a view.
    pub fn get_mut(&mut self, id: ViewId) -> &mut ViewTarget {
        match id {
            ViewId::Main3D => &mut self.main3d,
            ViewId::Main2D => &mut self.main2d,
            ViewId::Preview => &mut self.preview,
            ViewId::ExportPreview => &mut self.export_preview,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_rounds_up_to_the_quantum_and_clamps() {
        assert_eq!(alloc_side(1, 8192), 64);
        assert_eq!(alloc_side(64, 8192), 64);
        assert_eq!(alloc_side(65, 8192), 128);
        assert_eq!(alloc_side(1000, 8192), 1024);
        assert_eq!(alloc_side(8191, 8192), 8192);
        assert_eq!(alloc_side(8192, 8192), 8192);
        assert_eq!(alloc_side(u32::MAX, 8192), 8192);
        // A device limit below the quantum still yields a valid (quantum) allocation.
        assert_eq!(alloc_side(10, 32), 64);
    }

    #[test]
    fn used_size_is_at_least_one_pixel_and_at_most_the_device_limit() {
        assert_eq!(used_side(0, 8192), 1);
        assert_eq!(used_side(500, 8192), 500);
        assert_eq!(used_side(20000, 8192), 8192);
        for px in [1u32, 63, 64, 65, 700, 4097, 8192] {
            assert!(alloc_side(used_side(px, 8192), 8192) >= used_side(px, 8192));
        }
    }
}
