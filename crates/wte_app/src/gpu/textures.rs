//! GPU copy of the composite of one texture set (ARCHITECTURE.md 8.4): CPU-authoritative dirty-rect uploads into
//! level 0 and a box-filtered mip chain rendered with the renderer's own blit pipeline.

use eframe::wgpu;
use wte_core::IRect;

use crate::gpu::renderer::SetUniforms;
use crate::gpu::{Gpu, Renderer};

/// Largest number of bytes handed to one `write_texture` call: big dirty rects are uploaded in row bands so the
/// staging memory wgpu allocates per call stays bounded (a full 8192^2 upload would otherwise stage 256 MiB at once).
pub(crate) const UPLOAD_CHUNK_BYTES: usize = 16 << 20;

/// Number of levels of a full mip chain: `floor(log2(max(w, h))) + 1`.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// Size of mip level `level` of a `width x height` texture (each side halved, at least 1).
pub(crate) fn mip_size(width: u32, height: u32, level: u32) -> [u32; 2] {
    [(width >> level).max(1), (height >> level).max(1)]
}

/// Splits `r` into row bands of at most `max_bytes` (at least one row each); empty rects give nothing.
pub(crate) fn upload_bands(r: IRect, max_bytes: usize) -> Vec<IRect> {
    if r.is_empty() {
        return Vec::new();
    }
    let row_bytes = r.width() as usize * 4;
    let rows = (max_bytes / row_bytes.max(1)).max(1) as i64;
    let mut out = Vec::new();
    let mut y = r.y0 as i64;
    while y < r.y1 as i64 {
        let y1 = (y + rows).min(r.y1 as i64);
        out.push(IRect::new(r.x0, y as i32, r.x1, y1 as i32));
        y = y1;
    }
    out
}

/// Byte offset of texel `(x, y)` in a tightly packed RGBA8 buffer of width `width`.
fn texel_offset(x: i32, y: i32, width: u32) -> u64 {
    ((y as u64) * (width as u64) + x as u64) * 4
}

/// GPU copy of the composite of one set: `Rgba8Unorm`, full mip chain, `TEXTURE_BINDING | COPY_DST |
/// RENDER_ATTACHMENT`, per-mip views and the bind group of the set.
pub struct SetTexture {
    texture: wgpu::Texture,
    size: [u32; 2],
    /// Single-level views, index = mip level (render targets of the blit).
    mip_views: Vec<wgpu::TextureView>,
    /// `blit_groups[i]` samples level `i` (the source when rendering level `i + 1`).
    blit_groups: Vec<wgpu::BindGroup>,
    /// Group 1 of the mesh / UV pipelines: uniforms, the full-chain view and both samplers.
    bind_group: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
}

impl SetTexture {
    /// Creates the texture and uploads the whole composite (`size[0] * size[1] * 4` bytes). The mip chain is not
    /// generated here: the caller records `regenerate_mips` (GpuScene does so for every new set).
    pub fn new(gpu: &Gpu, renderer: &Renderer, size: [u32; 2], composite: &[u8]) -> SetTexture {
        let size = [size[0].max(1), size[1].max(1)];
        let mip_count = mip_level_count(size[0], size[1]);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("set_texture"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Gpu::COLOR_FORMAT,
            // COPY_SRC only in tests (GPU read-back of mip levels).
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | if cfg!(test) { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() },
            view_formats: &[],
        });
        let mip_views: Vec<wgpu::TextureView> = (0..mip_count)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("set_texture_mip"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let blit_groups = mip_views[..mip_views.len() - 1].iter().map(|v| renderer.create_blit_group(gpu, v)).collect();
        let full_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uniforms = SetUniforms {
            tex_size: [size[0] as f32, size[1] as f32, 1.0 / size[0] as f32, 1.0 / size[1] as f32],
            base_color: [1.0; 4],
            // x: has texture, y: mip level count (nearest filtering picks its level in the shader).
            params: [1, mip_count, 0, 0],
        };
        let (buffer, bind_group) = renderer.create_set_group(gpu, &full_view, &uniforms);
        let tex = SetTexture { texture, size, mip_views, blit_groups, bind_group, _uniforms: buffer };
        tex.upload(&gpu.queue, composite, IRect::from_size(size[0], size[1]));
        tex
    }

    /// Uploads the sub-rect `r` of level 0; `composite` is the whole buffer (layout in section 8.4). The rect is
    /// clipped to the texture and uploaded in row bands of at most `UPLOAD_CHUNK_BYTES`.
    pub fn upload(&self, queue: &wgpu::Queue, composite: &[u8], r: IRect) {
        let [w, h] = self.size;
        if composite.len() != w as usize * h as usize * 4 {
            log::error!("set texture upload skipped: {} bytes for a {w}x{h} texture", composite.len());
            return;
        }
        let r = r.intersect(&IRect::from_size(w, h));
        for band in upload_bands(r, UPLOAD_CHUNK_BYTES) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: band.x0 as u32, y: band.y0 as u32, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                composite,
                wgpu::TexelCopyBufferLayout {
                    offset: texel_offset(band.x0, band.y0, w),
                    bytes_per_row: Some(w * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d { width: band.width(), height: band.height(), depth_or_array_layers: 1 },
            );
        }
    }

    /// Re-renders mip levels `1..` from level 0 with the blit pipeline of the renderer.
    pub fn regenerate_mips(&self, _gpu: &Gpu, renderer: &Renderer, encoder: &mut wgpu::CommandEncoder) {
        for (level, target) in self.mip_views.iter().enumerate().skip(1) {
            let [w, h] = mip_size(self.size[0], self.size[1], level as u32);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mip_blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
            pass.set_pipeline(renderer.blit_pipeline());
            pass.set_bind_group(0, &self.blit_groups[level - 1], &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Size of level 0.
    pub(crate) fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Group 1 of the mesh and UV pipelines.
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// The texture itself (GPU read-back in tests).
    #[cfg(test)]
    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Releases the GPU memory now instead of when the last bind group referencing it is dropped.
    pub(crate) fn destroy(&self) {
        self.texture.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_length() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 1), 2);
        assert_eq!(mip_level_count(3, 3), 2);
        assert_eq!(mip_level_count(4, 4), 3);
        assert_eq!(mip_level_count(1024, 1024), 11);
        assert_eq!(mip_level_count(1024, 512), 11);
        assert_eq!(mip_level_count(1000, 7), 10);
        assert_eq!(mip_level_count(8192, 8192), 14);
        assert_eq!(mip_level_count(0, 0), 1);
    }

    #[test]
    fn mip_sizes_halve_down_to_one() {
        assert_eq!(mip_size(1024, 256, 0), [1024, 256]);
        assert_eq!(mip_size(1024, 256, 1), [512, 128]);
        assert_eq!(mip_size(1024, 256, 8), [4, 1]);
        assert_eq!(mip_size(1024, 256, 10), [1, 1]);
        assert_eq!(mip_size(5, 3, 1), [2, 1]);
        // The last level of every chain is 1x1.
        for (w, h) in [(1u32, 1u32), (7, 3), (1000, 7), (8192, 4096)] {
            assert_eq!(mip_size(w, h, mip_level_count(w, h) - 1), [1, 1]);
        }
    }

    #[test]
    fn upload_bands_cover_the_rect_exactly_and_respect_the_budget() {
        let r = IRect::new(3, 5, 103, 1005); // 100 x 1000, 400 bytes per row
        let bands = upload_bands(r, 4000); // 10 rows per band
        assert_eq!(bands.len(), 100);
        let mut y = r.y0;
        for b in &bands {
            assert_eq!((b.x0, b.x1), (r.x0, r.x1));
            assert_eq!(b.y0, y);
            assert!(b.height() >= 1 && b.area() as usize * 4 <= 4000);
            y = b.y1;
        }
        assert_eq!(y, r.y1);

        // A single row larger than the budget still goes out whole.
        let wide = IRect::new(0, 0, 10_000, 3);
        let bands = upload_bands(wide, 1024);
        assert_eq!(bands, vec![IRect::new(0, 0, 10_000, 1), IRect::new(0, 1, 10_000, 2), IRect::new(0, 2, 10_000, 3)]);

        // Small rects are one band; empty rects none.
        assert_eq!(upload_bands(IRect::new(1, 1, 9, 9), UPLOAD_CHUNK_BYTES), vec![IRect::new(1, 1, 9, 9)]);
        assert!(upload_bands(IRect::EMPTY, UPLOAD_CHUNK_BYTES).is_empty());

        // A full 8192^2 upload is split into 16 MiB bands.
        let full = upload_bands(IRect::from_size(8192, 8192), UPLOAD_CHUNK_BYTES);
        assert_eq!(full.len(), 16);
        assert!(full.iter().all(|b| b.area() as usize * 4 <= UPLOAD_CHUNK_BYTES));
    }

    #[test]
    fn texel_offsets_address_the_rect_in_place() {
        assert_eq!(texel_offset(0, 0, 640), 0);
        assert_eq!(texel_offset(3, 0, 640), 12);
        assert_eq!(texel_offset(3, 2, 640), (2 * 640 + 3) * 4);
        // No overflow at the largest texture.
        assert_eq!(texel_offset(8191, 8191, 8192), (8191u64 * 8192 + 8191) * 4);
    }
}
