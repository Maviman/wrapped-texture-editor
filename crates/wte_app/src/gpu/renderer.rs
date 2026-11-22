//! Pipelines and per-view rendering (ARCHITECTURE.md 8.4). Every `render_*` call writes the shared uniform buffer,
//! records one render pass (viewport and scissor = the used size, 4x MSAA resolved into the view's texture) and
//! submits immediately, so one uniform buffer serves all views.

use std::num::NonZeroU64;

use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::gpu::scene::DrawGroup;
use crate::gpu::{Gpu, GpuScene, View3dParams, ViewTarget, ViewUvParams};
use crate::state::Backdrop;

const MESH_WGSL: &str = include_str!("shaders/mesh.wgsl");
const LINES_WGSL: &str = include_str!("shaders/lines.wgsl");
const UVVIEW_WGSL: &str = include_str!("shaders/uvview.wgsl");
const BACKGROUND_WGSL: &str = include_str!("shaders/background.wgsl");
const BLIT_WGSL: &str = include_str!("shaders/blit.wgsl");

/// `ViewUniforms.params.y` flags.
pub(crate) const FLAG_LIT: u32 = 1;
pub(crate) const FLAG_ALPHA_CHECKER: u32 = 2;
pub(crate) const FLAG_NEAREST: u32 = 4;
pub(crate) const FLAG_MATTE: u32 = 8;

/// Ambient strength of the hemisphere light (`ViewUniforms.light_dir.w`).
const AMBIENT: f32 = 0.3;

/// Group 0 of every view pipeline (vec4 / mat4 only, matching `struct ViewUniforms` of the shaders).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniforms {
    pub view_proj: [[f32; 4]; 4],
    /// xyz
    pub eye: [f32; 4],
    /// xyz = key light (world, attached to the camera), w = ambient
    pub light_dir: [f32; 4],
    /// w, h, 1/w, 1/h in physical pixels
    pub viewport: [f32; 4],
    /// xyz = centre (world), w = radius (world); w == 0 => hidden
    pub brush: [f32; 4],
    /// x = hardness, yzw = side normal of the ring (unit; zero = the ring on every face)
    pub brush_aux: [f32; 4],
    /// rgb = matte colour (sRGB-encoded 0..1), used when the MATTE flag is set
    pub matte: [f32; 4],
    /// x = ShadingMode, y = flags, z = Backdrop
    pub params: [u32; 4],
}

/// Group 1 uniforms: one per texture set (`has_texture: 1`) and one per material for untextured drawing.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SetUniforms {
    /// w, h, 1/w, 1/h in texels
    pub tex_size: [f32; 4],
    /// sRGB-encoded 0..1
    pub base_color: [f32; 4],
    /// x = has_texture
    pub params: [u32; 4],
}

/// One wire instance of `lines.wgsl` (28 bytes: `Float32x3, Float32x3, Unorm8x4`).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LineInstance {
    pub a: [f32; 3],
    pub b: [f32; 3],
    /// Straight-alpha RGBA8.
    pub color: [u8; 4],
}

fn viewport_vec(size: [u32; 2]) -> [f32; 4] {
    let w = size[0].max(1) as f32;
    let h = size[1].max(1) as f32;
    [w, h, 1.0 / w, 1.0 / h]
}

/// Key light direction for a camera, derived from its view-projection matrix: `normalize(eye_dir + 0.4 * up + 0.3 *
/// right)`, where `eye_dir` points from the scene towards the camera. Works for perspective and orthographic cameras.
pub(crate) fn key_light_dir(view_proj: Mat4) -> Vec3 {
    let inv = view_proj.inverse();
    if !inv.is_finite() {
        return Vec3::new(0.3, 0.4, 1.0).normalize();
    }
    let unproject = |x: f32, y: f32, z: f32| inv.project_point3(Vec3::new(x, y, z));
    let near = unproject(0.0, 0.0, 0.0);
    let forward = (unproject(0.0, 0.0, 1.0) - near).normalize_or_zero();
    let right_raw = (unproject(1.0, 0.0, 0.0) - near).normalize_or_zero();
    let up = right_raw.cross(forward).normalize_or_zero();
    let right = forward.cross(up).normalize_or_zero();
    let dir = (-forward + 0.4 * up + 0.3 * right).normalize_or_zero();
    if dir == Vec3::ZERO { Vec3::new(0.3, 0.4, 1.0).normalize() } else { dir }
}

/// The ring's side filter as written to `brush_aux.yzw` (FEATURE_BRUSH 5.11): the unit side normal, or zero = no
/// filter (`None`, or a normal that is zero or not finite, so a degenerate hit never hides the whole ring). The mesh
/// shader draws the ring only where `dot(normal, side) >= SIDE_MIN_DOT`, the rule of the Circular brush in the core.
fn side_normal_vec(side: Option<Vec3>) -> Vec3 {
    side.filter(|n| n.is_finite()).map(Vec3::normalize_or_zero).unwrap_or(Vec3::ZERO)
}

/// Uniforms of a 3D render.
pub(crate) fn view_uniforms_3d(p: &View3dParams, size: [u32; 2]) -> ViewUniforms {
    let mut flags = 0;
    if p.lit {
        flags |= FLAG_LIT;
    }
    if p.alpha_checker {
        flags |= FLAG_ALPHA_CHECKER;
    }
    if p.nearest {
        flags |= FLAG_NEAREST;
    }
    let matte = match p.matte {
        Some(rgb) => {
            flags |= FLAG_MATTE;
            [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0, 1.0]
        }
        None => [0.0; 4],
    };
    let (brush, brush_aux) = match &p.brush {
        Some(b) if b.radius > 0.0 && b.radius.is_finite() && b.center.is_finite() => {
            let side = side_normal_vec(b.side_normal);
            (b.center.extend(b.radius).to_array(), [b.hardness.clamp(0.0, 1.0), side.x, side.y, side.z])
        }
        _ => ([0.0; 4], [0.0; 4]),
    };
    ViewUniforms {
        view_proj: p.view_proj.to_cols_array_2d(),
        eye: p.eye.extend(1.0).to_array(),
        light_dir: key_light_dir(p.view_proj).extend(AMBIENT).to_array(),
        viewport: viewport_vec(size),
        brush,
        brush_aux,
        matte,
        params: [p.shading as u32, flags, p.backdrop as u32, 0],
    }
}

/// Uniforms of a backdrop-only render.
pub(crate) fn view_uniforms_backdrop(backdrop: Backdrop, size: [u32; 2]) -> ViewUniforms {
    ViewUniforms {
        view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        eye: [0.0; 4],
        light_dir: [0.0; 4],
        viewport: viewport_vec(size),
        brush: [0.0; 4],
        brush_aux: [0.0; 4],
        matte: [0.0; 4],
        params: [0, 0, backdrop as u32, 0],
    }
}

/// The affine that maps a fragment position (physical pixels, origin top-left of the target) to UV:
/// `uv = center_uv + (frag - size / 2) / (px_per_texel * tex)` (the inverse of `UvViewState::uv_to_screen`).
pub(crate) fn uv_from_frag(center_uv: Vec2, px_per_texel: f32, tex: [u32; 2], size: [u32; 2]) -> Mat4 {
    let ppt = px_per_texel.max(1e-6);
    let sx = 1.0 / (ppt * tex[0].max(1) as f32);
    let sy = 1.0 / (ppt * tex[1].max(1) as f32);
    let half = Vec2::new(size[0] as f32, size[1] as f32) * 0.5;
    Mat4::from_cols(
        Vec4::new(sx, 0.0, 0.0, 0.0),
        Vec4::new(0.0, sy, 0.0, 0.0),
        Vec4::ZERO,
        Vec4::new(center_uv.x - half.x * sx, center_uv.y - half.y * sy, 0.0, 1.0),
    )
}

/// UV -> clip matrix of the UV wireframe (inverse mapping of `uv_from_frag`, then pixels -> NDC with Y up); the
/// output depth is a constant 0.5.
pub(crate) fn uv_to_clip(center_uv: Vec2, px_per_texel: f32, tex: [u32; 2], size: [u32; 2]) -> Mat4 {
    let ppt = px_per_texel.max(1e-6);
    let w = size[0].max(1) as f32;
    let h = size[1].max(1) as f32;
    let kx = tex[0].max(1) as f32 * ppt * 2.0 / w;
    let ky = tex[1].max(1) as f32 * ppt * 2.0 / h;
    Mat4::from_cols(
        Vec4::new(kx, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -ky, 0.0, 0.0),
        Vec4::ZERO,
        Vec4::new(-center_uv.x * kx, center_uv.y * ky, 0.5, 1.0),
    )
}

/// Uniforms of a UV render: `view_proj` = `uv_from_frag` for `uvview.wgsl`.
pub(crate) fn view_uniforms_uv(p: &ViewUvParams, tex: [u32; 2], size: [u32; 2]) -> ViewUniforms {
    let flags = if p.alpha_checker { FLAG_ALPHA_CHECKER } else { 0 };
    ViewUniforms {
        view_proj: uv_from_frag(p.center_uv, p.px_per_texel, tex, size).to_cols_array_2d(),
        params: [0, flags, Backdrop::Dark as u32, 0],
        ..view_uniforms_backdrop(Backdrop::Dark, size)
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages, size: usize) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size as u64),
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// How a view pipeline treats depth.
#[derive(Copy, Clone)]
enum DepthMode {
    /// `Less`, write on (the mesh).
    Opaque,
    /// `LessEqual`, no write (3D wires).
    Overlay,
    /// `Always`, no write (backdrop, UV view, UV wires).
    Ignore,
}

/// Everything that differs between the view pipelines.
struct PipelineSpec<'a> {
    label: &'a str,
    module: &'a wgpu::ShaderModule,
    layout: &'a wgpu::PipelineLayout,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    topology: wgpu::PrimitiveTopology,
    depth: DepthMode,
    blend: Option<wgpu::BlendState>,
}

fn view_pipeline(device: &wgpu::Device, samples: u32, spec: PipelineSpec<'_>) -> wgpu::RenderPipeline {
    let (write, compare) = match spec.depth {
        DepthMode::Opaque => (true, wgpu::CompareFunction::Less),
        DepthMode::Overlay => (false, wgpu::CompareFunction::LessEqual),
        DepthMode::Ignore => (false, wgpu::CompareFunction::Always),
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.module,
            entry_point: Some("vs_main"),
            buffers: spec.buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: spec.topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Gpu::DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(wgpu::FragmentState {
            module: spec.module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: Gpu::COLOR_FORMAT,
                blend: spec.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Straight-alpha blending of the colour that leaves the target alpha untouched (it stays 1).
const LINE_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const LINE_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4];

/// Pipelines, bind group layouts, samplers, uniform buffers, the mip blit pipeline and the dummy white texture.
pub struct Renderer {
    set_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    blit_sampler: wgpu::Sampler,
    blit_pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    lines3d_pipeline: wgpu::RenderPipeline,
    lines_uv_pipeline: wgpu::RenderPipeline,
    uvview_pipeline: wgpu::RenderPipeline,
    /// Shared view uniforms (group 0) of every render call.
    view_buffer: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    /// Second group-0 uniforms of the UV view: the UV wireframe needs a UV-to-clip matrix in the same pass.
    uv_lines_buffer: wgpu::Buffer,
    uv_lines_group: wgpu::BindGroup,
    /// 1x1 opaque white texture of the untextured bind groups.
    _dummy_texture: wgpu::Texture,
    dummy_view: wgpu::TextureView,
}

impl Renderer {
    /// Creates every pipeline and shared GPU object.
    pub fn new(gpu: &Gpu) -> Renderer {
        let device = &gpu.device;
        let module = |label: &str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let mesh_module = module("mesh.wgsl", MESH_WGSL);
        let lines_module = module("lines.wgsl", LINES_WGSL);
        let uvview_module = module("uvview.wgsl", UVVIEW_WGSL);
        let background_module = module("background.wgsl", BACKGROUND_WGSL);
        let blit_module = module("blit.wgsl", BLIT_WGSL);

        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("view_layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                size_of::<ViewUniforms>(),
            )],
        });
        let set_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("set_layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT, size_of::<SetUniforms>()),
                texture_entry(1),
                // One sampler only: nearest filtering is a `textureLoad` in the shaders. A texture sampled with two
                // samplers cannot be expressed in GLSL (the GL fallback backend), pipeline creation fails there.
                sampler_entry(2),
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });

        let layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let view_only = layout("view_only_layout", &[Some(&view_layout)]);
        let view_and_set = layout("view_set_layout", &[Some(&view_layout), Some(&set_layout)]);
        let blit_pl = layout("blit_pipeline_layout", &[Some(&blit_layout)]);

        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<wte_core::RenderVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRIBUTES,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<LineInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &LINE_ATTRIBUTES,
        })];

        let fullscreen = |label, module, layout| PipelineSpec {
            label,
            module,
            layout,
            buffers: &[],
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth: DepthMode::Ignore,
            blend: None,
        };
        let samples = gpu.msaa;
        let background_pipeline = view_pipeline(device, samples, fullscreen("background", &background_module, &view_only));
        let uvview_pipeline = view_pipeline(device, samples, fullscreen("uvview", &uvview_module, &view_and_set));
        let mesh_pipeline = view_pipeline(
            device,
            samples,
            PipelineSpec {
                label: "mesh",
                module: &mesh_module,
                layout: &view_and_set,
                buffers: &mesh_buffers,
                topology: wgpu::PrimitiveTopology::TriangleList,
                depth: DepthMode::Opaque,
                blend: None,
            },
        );
        let lines = |label, depth| PipelineSpec {
            label,
            module: &lines_module,
            layout: &view_only,
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            depth,
            blend: Some(LINE_BLEND),
        };
        let lines3d_pipeline = view_pipeline(device, samples, lines("lines3d", DepthMode::Overlay));
        let lines_uv_pipeline = view_pipeline(device, samples, lines("lines_uv", DepthMode::Ignore));

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mip_blit"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Gpu::COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear_repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mip_blit"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let uniform_group = |label: &str| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(&view_uniforms_backdrop(Backdrop::Dark, [1, 1])),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &view_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            });
            (buffer, group)
        };
        let (view_buffer, view_group) = uniform_group("view_uniforms");
        let (uv_lines_buffer, uv_lines_group) = uniform_group("uv_lines_uniforms");

        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_white"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Gpu::COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            dummy_texture.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Renderer {
            set_layout,
            blit_layout,
            linear_sampler,

            blit_sampler,
            blit_pipeline,
            background_pipeline,
            mesh_pipeline,
            lines3d_pipeline,
            lines_uv_pipeline,
            uvview_pipeline,
            view_buffer,
            view_group,
            uv_lines_buffer,
            uv_lines_group,
            _dummy_texture: dummy_texture,
            dummy_view,
        }
    }

    /// Renders the model into `target` (backdrop, mesh, wireframe / seams, brush ring) and submits.
    pub fn render_3d(&mut self, gpu: &Gpu, scene: &GpuScene, target: &ViewTarget, p: &View3dParams) {
        let size = target.size_px();
        gpu.queue.write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&view_uniforms_3d(p, size)));
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render_3d") });
        {
            let mut pass = begin_view_pass(&mut encoder, target, "render_3d");
            pass.set_bind_group(0, &self.view_group, &[]);
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);

            if let Some((vertices, indices)) = scene.mesh_buffers() {
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                for draw in scene.draws() {
                    let group = match draw.group {
                        DrawGroup::Set(s) => scene.set_texture(s).map(|t| t.bind_group()),
                        DrawGroup::Untextured(m) => scene.untextured_group(m),
                    };
                    let Some(group) = group else { continue };
                    pass.set_bind_group(1, group, &[]);
                    pass.draw_indexed(draw.tris.start * 3..draw.tris.end * 3, 0, 0..1);
                }
            }

            let wires = if p.wireframe {
                scene.wire_all()
            } else if p.seams {
                scene.wire_seams()
            } else {
                None
            };
            if let Some((buffer, count)) = wires {
                pass.set_pipeline(&self.lines3d_pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..4, 0..count);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// Renders the UV view of one set into `target` and submits.
    pub fn render_uv(&mut self, gpu: &Gpu, scene: &GpuScene, target: &ViewTarget, p: &ViewUvParams) {
        let Some(set) = scene.set_texture(p.set.index()) else {
            self.render_backdrop(gpu, target, Backdrop::Dark);
            return;
        };
        let size = target.size_px();
        let tex = set.size();
        let uniforms = view_uniforms_uv(p, tex, size);
        gpu.queue.write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&uniforms));
        let wires = if p.uv_wire { scene.wire_uv(p.set.index()) } else { None };
        if wires.is_some() {
            let lines = ViewUniforms {
                view_proj: uv_to_clip(p.center_uv, p.px_per_texel, tex, size).to_cols_array_2d(),
                ..uniforms
            };
            gpu.queue.write_buffer(&self.uv_lines_buffer, 0, bytemuck::bytes_of(&lines));
        }
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render_uv") });
        {
            let mut pass = begin_view_pass(&mut encoder, target, "render_uv");
            pass.set_pipeline(&self.uvview_pipeline);
            pass.set_bind_group(0, &self.view_group, &[]);
            pass.set_bind_group(1, set.bind_group(), &[]);
            pass.draw(0..3, 0..1);
            if let Some((buffer, count)) = wires {
                pass.set_pipeline(&self.lines_uv_pipeline);
                pass.set_bind_group(0, &self.uv_lines_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..4, 0..count);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// Renders only the backdrop into `target` (no document) and submits.
    pub fn render_backdrop(&mut self, gpu: &Gpu, target: &ViewTarget, backdrop: Backdrop) {
        let size = target.size_px();
        gpu.queue.write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&view_uniforms_backdrop(backdrop, size)));
        let mut encoder =
            gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render_backdrop") });
        {
            let mut pass = begin_view_pass(&mut encoder, target, "render_backdrop");
            pass.set_bind_group(0, &self.view_group, &[]);
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// Group 1 for a texture (a set texture, or the dummy white texture for untextured drawing).
    pub(crate) fn create_set_group(
        &self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        uniforms: &SetUniforms,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("set_uniforms"),
            contents: bytemuck::bytes_of(uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("set_group"),
            layout: &self.set_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
            ],
        });
        (buffer, group)
    }

    /// Group 1 of untextured drawing: the dummy white texture and `SetUniforms { base_color, has_texture: 0 }`.
    pub(crate) fn create_untextured_group(&self, gpu: &Gpu, base_color: [f32; 4]) -> (wgpu::Buffer, wgpu::BindGroup) {
        let base_color = base_color.map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 1.0 });
        let uniforms = SetUniforms { tex_size: [1.0; 4], base_color, params: [0; 4] };
        self.create_set_group(gpu, &self.dummy_view, &uniforms)
    }

    /// Bind group of the mip blit: one single-level source view and the bilinear clamp sampler.
    pub(crate) fn create_blit_group(&self, gpu: &Gpu, src: &wgpu::TextureView) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mip_blit"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.blit_sampler) },
            ],
        })
    }

    /// The mip blit pipeline (`blit.wgsl`, no blending, no depth).
    pub(crate) fn blit_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.blit_pipeline
    }
}

/// One render pass into a view target: MSAA colour (`Gpu::msaa` samples) resolved into the target's texture, or with
/// one sample straight into it; cleared depth, viewport and scissor = the used size.
fn begin_view_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &'e ViewTarget,
    label: &'e str,
) -> wgpu::RenderPass<'e> {
    let [w, h] = target.size_px();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target.msaa_view().unwrap_or(target.resolve_view()),
            depth_slice: None,
            resolve_target: target.msaa_view().map(|_| target.resolve_view()),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                // Without MSAA the colour attachment is the resolve texture itself: it must be stored.
                store: if target.msaa_view().is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: target.depth_view(),
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
    pass.set_scissor_rect(0, 0, w, h);
    pass
}

#[cfg(test)]
mod tests {
    use eframe::wgpu::naga;

    use super::*;
    use crate::gpu::BrushRing;
    use crate::state::ShadingMode;

    const SHADERS: [(&str, &str); 5] = [
        ("mesh.wgsl", MESH_WGSL),
        ("lines.wgsl", LINES_WGSL),
        ("uvview.wgsl", UVVIEW_WGSL),
        ("background.wgsl", BACKGROUND_WGSL),
        ("blit.wgsl", BLIT_WGSL),
    ];

    fn parse(name: &str, source: &str) -> naga::Module {
        naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)))
    }

    #[test]
    fn every_shader_is_valid_wgsl() {
        for (name, source) in SHADERS {
            let module = parse(name, source);
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            for entry in ["vs_main", "fs_main"] {
                assert!(module.entry_points.iter().any(|e| e.name == entry), "{name} lacks {entry}");
            }
        }
    }

    /// Size and member offsets of a WGSL struct.
    fn wgsl_struct(module: &naga::Module, name: &str) -> Option<(u32, Vec<(String, u32)>)> {
        module.types.iter().find_map(|(_, ty)| match &ty.inner {
            naga::TypeInner::Struct { members, span } if ty.name.as_deref() == Some(name) => {
                Some((*span, members.iter().map(|m| (m.name.clone().unwrap_or_default(), m.offset)).collect()))
            }
            _ => None,
        })
    }

    #[test]
    fn uniform_structs_match_the_wgsl_layout() {
        let view_expected = vec![
            ("view_proj".to_owned(), std::mem::offset_of!(ViewUniforms, view_proj) as u32),
            ("eye".to_owned(), std::mem::offset_of!(ViewUniforms, eye) as u32),
            ("light_dir".to_owned(), std::mem::offset_of!(ViewUniforms, light_dir) as u32),
            ("viewport".to_owned(), std::mem::offset_of!(ViewUniforms, viewport) as u32),
            ("brush".to_owned(), std::mem::offset_of!(ViewUniforms, brush) as u32),
            ("brush_aux".to_owned(), std::mem::offset_of!(ViewUniforms, brush_aux) as u32),
            ("matte".to_owned(), std::mem::offset_of!(ViewUniforms, matte) as u32),
            ("params".to_owned(), std::mem::offset_of!(ViewUniforms, params) as u32),
        ];
        let set_expected = vec![
            ("tex_size".to_owned(), std::mem::offset_of!(SetUniforms, tex_size) as u32),
            ("base_color".to_owned(), std::mem::offset_of!(SetUniforms, base_color) as u32),
            ("params".to_owned(), std::mem::offset_of!(SetUniforms, params) as u32),
        ];
        assert_eq!(size_of::<ViewUniforms>(), 176);
        assert_eq!(size_of::<SetUniforms>(), 48);
        for (name, source) in SHADERS {
            let module = parse(name, source);
            if let Some((span, members)) = wgsl_struct(&module, "ViewUniforms") {
                assert_eq!(span as usize, size_of::<ViewUniforms>(), "{name}: ViewUniforms size");
                assert_eq!(members, view_expected, "{name}: ViewUniforms members");
            } else {
                assert_eq!(name, "blit.wgsl", "{name} has no ViewUniforms");
            }
            if let Some((span, members)) = wgsl_struct(&module, "SetUniforms") {
                assert_eq!(span as usize, size_of::<SetUniforms>(), "{name}: SetUniforms size");
                assert_eq!(members, set_expected, "{name}: SetUniforms members");
            }
        }
    }

    #[test]
    fn vertex_layouts_match_the_core_and_the_shaders() {
        assert_eq!(size_of::<wte_core::RenderVertex>(), 32);
        assert_eq!(MESH_ATTRIBUTES.map(|a| a.offset), [0, 12, 24]);
        assert_eq!(size_of::<LineInstance>(), 28);
        assert_eq!(LINE_ATTRIBUTES.map(|a| a.offset), [0, 12, 24]);
        assert_eq!(std::mem::offset_of!(LineInstance, color), 24);
    }

    #[test]
    fn key_light_follows_the_camera() {
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective(0.8, 1.5, 0.1, 100.0);
        let l = key_light_dir(proj * view);
        let expected = Vec3::new(0.3, 0.4, 1.0).normalize();
        assert!((l - expected).length() < 1e-3, "{l} != {expected}");

        // Camera looking down -X from +X: eye_dir = +X, up = +Y, right = -Z.
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO, Vec3::Y);
        let l = key_light_dir(proj * view);
        let expected = Vec3::new(1.0, 0.4, -0.3).normalize();
        assert!((l - expected).length() < 1e-3, "{l} != {expected}");

        // Orthographic cameras work the same way; a singular matrix gives a fixed direction.
        let ortho = glam::camera::rh::proj::directx::orthographic(-2.0, 2.0, -1.0, 1.0, 0.1, 10.0);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        assert!((key_light_dir(ortho * view) - Vec3::new(0.3, 0.4, 1.0).normalize()).length() < 1e-3);
        assert!(key_light_dir(Mat4::ZERO).is_normalized());
    }

    #[test]
    fn uv_matrices_are_inverse_mappings() {
        let tex = [1024, 512];
        let size = [800, 600];
        let center = Vec2::new(0.4, 0.55);
        let ppt = 0.75;
        let from_frag = uv_from_frag(center, ppt, tex, size);
        let to_clip = uv_to_clip(center, ppt, tex, size);
        // The target centre shows `center_uv`.
        let c = from_frag * Vec4::new(400.0, 300.0, 0.0, 1.0);
        assert!((c.truncate().truncate() - center).length() < 1e-6);
        for uv in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.25, 0.8), Vec2::new(-0.3, 1.7)] {
            let clip = to_clip * Vec4::new(uv.x, uv.y, 0.0, 1.0);
            assert_eq!(clip.w, 1.0);
            assert!((clip.z - 0.5).abs() < 1e-6);
            // NDC -> pixels (Y down), then back to UV.
            let px = Vec2::new((clip.x + 1.0) * 0.5 * 800.0, (1.0 - clip.y) * 0.5 * 600.0);
            let back = from_frag * Vec4::new(px.x, px.y, 0.0, 1.0);
            assert!((back.truncate().truncate() - uv).length() < 1e-5, "{uv} -> {px} -> {back}");
        }
        // One texel spans `px_per_texel` pixels; the shader reads the scale back from [0][0].
        let m = from_frag.to_cols_array_2d();
        assert!((1.0 / (m[0][0] * tex[0] as f32) - ppt).abs() < 1e-5);
        // Same convention as UvViewState::uv_to_screen: rect.center() + (uv - center) * tex * zoom.
        let uv = Vec2::new(0.9, 0.1);
        let clip = to_clip * Vec4::new(uv.x, uv.y, 0.0, 1.0);
        let px = Vec2::new((clip.x + 1.0) * 400.0, (1.0 - clip.y) * 300.0);
        let expected = Vec2::new(400.0, 300.0) + (uv - center) * Vec2::new(1024.0, 512.0) * ppt;
        assert!((px - expected).length() < 1e-3);
    }

    #[test]
    fn uniforms_of_a_3d_render() {
        let params = View3dParams {
            view_proj: Mat4::IDENTITY,
            eye: Vec3::new(1.0, 2.0, 3.0),
            shading: ShadingMode::UvChecker,
            lit: true,
            alpha_checker: true,
            matte: Some([255, 0, 51]),
            wireframe: false,
            seams: false,
            nearest: true,
            backdrop: Backdrop::Checker,
            brush: Some(BrushRing { center: Vec3::new(0.5, 0.0, 0.0), radius: 0.25, hardness: 0.6, side_normal: None }),
        };
        let u = view_uniforms_3d(&params, [640, 480]);
        assert_eq!(u.params, [2, FLAG_LIT | FLAG_ALPHA_CHECKER | FLAG_NEAREST | FLAG_MATTE, 3, 0]);
        assert_eq!(u.viewport, [640.0, 480.0, 1.0 / 640.0, 1.0 / 480.0]);
        assert_eq!(u.eye, [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(u.brush, [0.5, 0.0, 0.0, 0.25]);
        assert_eq!(u.brush_aux[0], 0.6);
        assert_eq!(u.matte, [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(u.light_dir[3], AMBIENT);

        let plain = View3dParams { lit: false, alpha_checker: false, matte: None, nearest: false, brush: None, ..params };
        let u = view_uniforms_3d(&plain, [0, 0]);
        assert_eq!(u.params[1], 0);
        assert_eq!(u.brush[3], 0.0, "no brush => ring hidden");
        assert_eq!(u.viewport, [1.0, 1.0, 1.0, 1.0], "zero sizes never divide by zero");

        let uv = view_uniforms_uv(
            &ViewUvParams {
                set: wte_core::TextureSetId(0),
                center_uv: Vec2::splat(0.5),
                px_per_texel: 2.0,
                uv_wire: true,
                alpha_checker: true,
            },
            [256, 256],
            [512, 512],
        );
        assert_eq!(uv.params[1], FLAG_ALPHA_CHECKER);
        assert_eq!(view_uniforms_backdrop(Backdrop::Light, [10, 10]).params[2], 1);
    }

    /// A14 (FEATURE_BRUSH 8.2): the ring's side normal is written into `brush_aux.yzw` as a unit vector, zeros without
    /// one; the uniform block keeps its 176 bytes and the size / hardness packing of `uniforms_of_a_3d_render`.
    #[test]
    fn the_side_normal_is_packed_into_brush_aux() {
        let ring = |side_normal: Option<Vec3>| BrushRing { center: Vec3::new(0.5, 0.0, 0.0), radius: 0.25, hardness: 0.6, side_normal };
        let params = |side: Option<Vec3>| View3dParams {
            view_proj: Mat4::IDENTITY,
            eye: Vec3::Z,
            shading: ShadingMode::Textured,
            lit: false,
            alpha_checker: false,
            matte: None,
            wireframe: false,
            seams: false,
            nearest: false,
            backdrop: Backdrop::Dark,
            brush: Some(ring(side)),
        };
        let aux = |side: Option<Vec3>| view_uniforms_3d(&params(side), [64, 64]).brush_aux;

        assert_eq!(aux(None), [0.6, 0.0, 0.0, 0.0], "no side normal: zeros = the ring on every face");
        assert_eq!(aux(Some(Vec3::Z)), [0.6, 0.0, 0.0, 1.0]);
        assert_eq!(aux(Some(Vec3::NEG_X)), [0.6, -1.0, 0.0, 0.0]);
        // A normal that is not unit length is normalised (the shader compares a dot product with -0.2).
        let a = aux(Some(Vec3::new(0.0, 3.0, 4.0)));
        assert_eq!(a[0], 0.6);
        assert!((Vec3::new(a[1], a[2], a[3]) - Vec3::new(0.0, 0.6, 0.8)).length() < 1e-6, "{a:?}");
        // A degenerate normal never hides the whole ring: it packs as "no filter".
        for bad in [Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 1.0), Vec3::new(f32::INFINITY, 0.0, 0.0)] {
            assert_eq!(aux(Some(bad)), [0.6, 0.0, 0.0, 0.0], "{bad}");
        }
        // The side normal changes nothing else: centre, radius, hardness and the rest of the block as without it.
        let with = view_uniforms_3d(&params(Some(Vec3::Y)), [64, 64]);
        let without = view_uniforms_3d(&params(None), [64, 64]);
        assert_eq!(with.brush, [0.5, 0.0, 0.0, 0.25]);
        assert_eq!(ViewUniforms { brush_aux: without.brush_aux, ..with }, without);
        // No ring: the whole of brush_aux is zero, whatever the side normal was.
        let hidden = View3dParams { brush: Some(BrushRing { radius: 0.0, ..ring(Some(Vec3::Z)) }), ..params(None) };
        assert_eq!(view_uniforms_3d(&hidden, [64, 64]).brush_aux, [0.0; 4]);
        assert_eq!(size_of::<ViewUniforms>(), 176, "the uniform block keeps its size");
        // The shader's threshold is the Circular brush's rule of the core (brush.rs SAME_SIDE_MIN_DOT = -0.2).
        assert!(MESH_WGSL.contains("const SIDE_MIN_DOT: f32 = -0.2;"));
        assert!(MESH_WGSL.contains("view.brush_aux.yzw"));
    }
}
