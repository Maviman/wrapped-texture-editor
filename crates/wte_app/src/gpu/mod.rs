//! GPU layer (ARCHITECTURE.md 8.4): device handles, offscreen view targets, set textures with mips, the document
//! scene and the renderer with its WGSL pipelines.

use std::sync::Arc;

use eframe::{egui, egui_wgpu, wgpu};

pub mod params;
pub mod renderer;
pub mod scene;
pub mod targets;
pub mod textures;

pub use params::{BrushRing, View3dParams, ViewUvParams};
pub use renderer::Renderer;
pub use scene::GpuScene;
pub use targets::{ViewTarget, ViewTargets};
pub use textures::SetTexture;

/// Handles to the wgpu device, its queue and egui's renderer (for registering view textures).
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub egui_renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    /// Largest side of the view targets: the device limit, at most 8192 (keeps their memory bounded).
    pub max_side: u32,
    /// Sample count of the view targets and the view pipelines: `Gpu::MSAA`, or 1 on DX12 (see `msaa_for`).
    pub msaa: u32,
}

impl Gpu {
    /// Format of set textures and view targets: sRGB-encoded bytes in a non-sRGB format (section 4.2).
    pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    /// Depth format of the view targets.
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
    /// Sample count of the view targets (anti-aliased edges and wires).
    pub const MSAA: u32 = 4;

    /// Clones the device, queue and renderer handles of eframe's render state.
    pub fn new(rs: &egui_wgpu::RenderState) -> Gpu {
        Gpu {
            device: rs.device.clone(),
            queue: rs.queue.clone(),
            egui_renderer: Arc::clone(&rs.renderer),
            max_side: rs.device.limits().max_texture_dimension_2d.min(8192),
            msaa: msaa_for(rs.adapter.get_info().backend),
        }
    }

    /// The limits the device was created with (buffer and texture sizes the scene must stay within).
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }
}

/// View-target sample count for a backend: `Gpu::MSAA`, except 1 on DX12, where a 4x MSAA pass with a resolve target
/// of 256 px and more loses the device on some drivers (seen with an AMD Radeon iGPU, driver 31.0.21924). DX12 is only
/// used when no Vulkan adapter is there (`main.rs` prefers Vulkan), so this costs anti-aliasing only on such machines.
pub fn msaa_for(backend: wgpu::Backend) -> u32 {
    if backend == wgpu::Backend::Dx12 { 1 } else { Gpu::MSAA }
}

/// Real-GPU checks: a headless device renders the sample sphere offscreen and the pixels are read back. Ignored by
/// default (they need a GPU); run with `cargo test -p wte_app --offline -- --ignored gpu::gpu_tests`. Images are written
/// to `target/selftest/gpu_probe_*.png` for inspection.
#[cfg(test)]
mod gpu_tests {
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use glam::{Vec2, Vec3};
    use wte_core::{Document, SampleKind, TextureSetId};

    use super::*;
    use crate::state::{Backdrop, ShadingMode};

    /// Minimal executor: wgpu's native futures resolve without an external reactor.
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
            label: Some("gpu_probe"),
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..base },
            ..Default::default()
        }))
        .expect("no wgpu device");
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, Gpu::COLOR_FORMAT, egui_wgpu::RendererOptions::default());
        Gpu {
            max_side: device.limits().max_texture_dimension_2d.min(8192),
            msaa: msaa_for(adapter.get_info().backend),
            device,
            queue,
            egui_renderer: Arc::new(egui::mutex::RwLock::new(egui_renderer)),
        }
    }

    /// Reads the used part of a view target back as tightly packed RGBA8.
    fn read_back(gpu: &Gpu, target: &ViewTarget) -> Vec<u8> {
        let [w, h] = target.size_px();
        let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_back"),
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

    fn save(name: &str, w: u32, h: u32, rgba: &[u8]) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/selftest");
        std::fs::create_dir_all(&dir).expect("create target/selftest");
        let png = wte_core::encode_png(w, h, rgba, wte_core::PngLevel::Fast).expect("encode png");
        std::fs::write(dir.join(format!("gpu_probe_{name}.png")), png).expect("write png");
    }

    fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    /// Per-channel tolerance of one step: GPUs differ in how they round floats to unorm bytes.
    fn close(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 1)
    }

    fn camera(doc: &Document, aspect: f32) -> (glam::Mat4, Vec3) {
        let bounds = doc.mesh.bounds;
        let (center, radius) = (bounds.center(), (bounds.diagonal() * 0.5).max(1e-6));
        let fov_y = std::f32::consts::FRAC_PI_4;
        let distance = radius / (fov_y * 0.5).sin() * 1.15;
        let eye = center + Vec3::new(0.0, 0.0, 1.0) * distance;
        let view = glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Y);
        let (near, far) = ((distance - 1.5 * radius).max(1e-3 * radius), distance + 1.5 * radius);
        (glam::camera::rh::proj::directx::perspective(fov_y, aspect, near, far) * view, eye)
    }

    fn params(doc: &Document, aspect: f32) -> View3dParams {
        let (view_proj, eye) = camera(doc, aspect);
        View3dParams {
            view_proj,
            eye,
            shading: ShadingMode::Textured,
            lit: false,
            alpha_checker: true,
            matte: None,
            wireframe: false,
            seams: false,
            nearest: false,
            backdrop: Backdrop::Dark,
            brush: None,
            all_objects: false,
            highlight: false,
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn renders_the_sample_sphere_offscreen() {
        let gpu = headless_gpu();
        let mut renderer = Renderer::new(&gpu);
        let mut targets = ViewTargets::new(&gpu);
        let mut doc = Document::sample(SampleKind::Sphere);
        let mut scene = GpuScene::new(&gpu, &renderer, &mut doc, 1);
        assert!(!scene.mips_stale(), "a new scene has complete mips");

        let (w, h) = (300u32, 200u32);
        let target = &mut targets.main3d;
        target.ensure_size(&gpu, [w, h]);
        assert_eq!(target.size_px(), [w, h]);
        let uv = target.uv_rect();
        assert!((uv.max.x - 300.0 / 320.0).abs() < 1e-6 && (uv.max.y - 200.0 / 256.0).abs() < 1e-6);

        // Backdrop only.
        renderer.render_backdrop(&gpu, target, Backdrop::Light);
        let img = read_back(&gpu, target);
        save("backdrop", w, h, &img);
        assert!(close(px(&img, w, 5, 5), [204, 207, 212, 255]), "{:?}", px(&img, w, 5, 5));

        // Unlit textured: the sphere centre shows composite bytes (the checker), the corner the dark backdrop.
        let p = params(&doc, w as f32 / h as f32);
        renderer.render_3d(&gpu, &scene, target, &p);
        let img = read_back(&gpu, target);
        save("unlit", w, h, &img);
        assert!(close(px(&img, w, 2, 2), [30, 31, 34, 255]), "corner {:?} is not the dark backdrop", px(&img, w, 2, 2));
        // Inside the sphere most pixels are exact texture colours (cell interiors), and both checker tones appear.
        let composite = doc.composite(TextureSetId(0)).unwrap();
        let mut palette: Vec<[u8; 4]> = composite.data.chunks(4).map(|c| [c[0], c[1], c[2], 255]).collect();
        palette.sort_unstable();
        palette.dedup();
        let region: Vec<[u8; 4]> =
            (h / 2 - 30..h / 2 + 30).flat_map(|y| (w / 2 - 30..w / 2 + 30).map(move |x| (x, y))).map(|(x, y)| px(&img, w, x, y)).collect();
        let exact = region.iter().filter(|c| palette.iter().any(|p| close(**c, *p))).count();
        assert!(exact * 2 > region.len(), "only {exact} of {} sphere pixels are texture colours", region.len());
        let tones: Vec<&[u8; 4]> = palette.iter().filter(|p| region.iter().any(|c| close(*c, **p))).collect();
        assert!(tones.len() >= 2, "the sphere shows only {tones:?}");
        // The checker shows more than one colour across the sphere.
        let row: Vec<[u8; 4]> = (w / 2 - 50..w / 2 + 50).map(|x| px(&img, w, x, h / 2)).collect();
        assert!(row.iter().any(|c| c != &row[0]), "the sphere is flat-coloured");
        assert!(img.chunks(4).all(|c| c[3] == 255), "the target alpha is always 1");

        // Lit + wireframe + brush ring + clay: differs from unlit, still opaque.
        let lit = View3dParams {
            lit: true,
            wireframe: true,
            shading: ShadingMode::Clay,
            backdrop: Backdrop::Gradient,
            brush: Some(BrushRing { center: Vec3::new(0.0, 0.0, 1.0), radius: 0.3, hardness: 0.5, side_normal: None }),
            ..p.clone()
        };
        renderer.render_3d(&gpu, &scene, target, &lit);
        let lit_img = read_back(&gpu, target);
        save("lit_clay_wire", w, h, &lit_img);
        assert_ne!(lit_img, img);
        let checker = View3dParams { shading: ShadingMode::UvChecker, seams: true, lit: true, ..p.clone() };
        renderer.render_3d(&gpu, &scene, target, &checker);
        save("uv_checker_seams", w, h, &read_back(&gpu, target));

        // Document edits reach the GPU through sync: hiding "Base" leaves the empty paint layer, so the sphere shows
        // the transparency checker.
        let base = doc.sets[0].layers[0].id;
        doc.set_layer_visible(base, false).expect("hide base");
        scene.sync(&gpu, &renderer, &mut doc, false);
        assert!(!scene.mips_stale(), "no interaction running: mips regenerate at once");
        renderer.render_3d(&gpu, &scene, target, &p);
        let hidden = read_back(&gpu, target);
        save("base_hidden", w, h, &hidden);
        let c = px(&hidden, w, w / 2, h / 2);
        assert!(close(c, [209, 209, 209, 255]) || close(c, [158, 158, 158, 255]), "centre {c:?} is not the alpha checker");
        doc.set_layer_visible(base, true).expect("show base");

        // During an interaction (a pending slider edit) mips are throttled, a forced sync regenerates them.
        doc.set_layer_opacity(base, 0.5, wte_core::Commit::Preview).expect("preview opacity");
        assert!(doc.has_pending_edit());
        scene.sync(&gpu, &renderer, &mut doc, true);
        doc.set_layer_opacity(base, 0.25, wte_core::Commit::Preview).expect("preview opacity");
        scene.sync(&gpu, &renderer, &mut doc, false);
        assert!(scene.mips_stale(), "mips regenerated less than 100 ms after the previous chain during a drag");
        scene.sync(&gpu, &renderer, &mut doc, true);
        assert!(!scene.mips_stale());
        assert!(doc.cancel_pending_edit());
        scene.sync(&gpu, &renderer, &mut doc, false);
        assert!(!scene.mips_stale());
        renderer.render_3d(&gpu, &scene, target, &p);
        assert_eq!(read_back(&gpu, target), img, "after cancelling, the render is identical to the first one");
        // UV view of the set: centre shows the texture, far outside the [0,1] square the dim backdrop.
        let t2 = &mut targets.main2d;
        t2.ensure_size(&gpu, [256, 256]);
        let uvp = ViewUvParams {
            set: TextureSetId(0),
            center_uv: Vec2::splat(0.5),
            px_per_texel: 0.2,
            uv_wire: true,
            alpha_checker: true,
        };
        renderer.render_uv(&gpu, &scene, t2, &uvp);
        let uv_img = read_back(&gpu, t2);
        save("uv", 256, 256, &uv_img);
        assert!(close(px(&uv_img, 256, 1, 1), [19, 20, 22, 255]), "outside the UV square: {:?}", px(&uv_img, 256, 1, 1));
        let zoomed = ViewUvParams { px_per_texel: 20.0, uv_wire: false, ..uvp };
        renderer.render_uv(&gpu, &scene, t2, &zoomed);
        save("uv_zoomed", 256, 256, &read_back(&gpu, t2));
        // A set that does not exist renders the backdrop instead of failing.
        renderer.render_uv(&gpu, &scene, t2, &ViewUvParams { set: TextureSetId(9), ..uvp });

        // Resizing keeps the egui id; destroy frees it.
        let id = targets.main3d.egui_id;
        targets.main3d.ensure_size(&gpu, [1000, 700]);
        assert_eq!(targets.main3d.egui_id, id);
        assert_eq!(targets.main3d.size_px(), [1000, 700]);
        renderer.render_3d(&gpu, &scene, &targets.main3d, &p);
        let ViewTargets { main3d, main2d, preview, export_preview } = targets;
        for t in [main3d, main2d, preview, export_preview] {
            t.destroy(&gpu);
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll failed");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn mips_are_box_filtered() {
        let gpu = headless_gpu();
        let renderer = Renderer::new(&gpu);
        // 4x4: columns alternate black / white; level 1 (2x2) must be uniform mid grey, level 2 too.
        let mut data = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                let v = if x % 2 == 0 { 0 } else { 255 };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let tex = SetTexture::new(&gpu, &renderer, [4, 4], &data);
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        tex.regenerate_mips(&gpu, &renderer, &mut enc);
        gpu.queue.submit([enc.finish()]);

        let level_bytes = read_level(&gpu, &tex, 1);
        for c in level_bytes.chunks(4) {
            assert!((c[0] as i32 - 128).abs() <= 1, "level 1 texel {c:?} is not the 2x2 box average");
        }
        let level_bytes = read_level(&gpu, &tex, 2);
        assert!((level_bytes[0] as i32 - 128).abs() <= 1);
    }

    fn read_level(gpu: &Gpu, tex: &SetTexture, level: u32) -> Vec<u8> {
        let [w, h] = textures::mip_size(tex.size()[0], tex.size()[1], level);
        let padded = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex.texture(),
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([enc.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("map failed"));
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll failed");
        let mapped = buffer.get_mapped_range(..).expect("mapped range");
        let mut out = Vec::new();
        for row in mapped.chunks(padded as usize).take(h as usize) {
            out.extend_from_slice(&row[..(w * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }
}
