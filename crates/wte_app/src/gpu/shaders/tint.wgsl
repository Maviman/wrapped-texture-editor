// Selection tint of the 3D view (objects-tabs, ARCHITECTURE.md 8.4): the triangles of the selected visible objects are
// drawn once more over the mesh in a translucent accent colour. The depth is pulled towards the camera by the same
// amount as the wires (lines.wgsl), so the tint wins the LessEqual test against the mesh's own depth and loses against
// everything in front of it. Like the wire colours, the colour is sRGB-encoded bytes / 255 written into the Rgba8Unorm
// target; the blend leaves the target alpha alone.

struct ViewUniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
    viewport: vec4<f32>,
    brush: vec4<f32>,
    brush_aux: vec4<f32>,
    matte: vec4<f32>,
    params: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: ViewUniforms;

const DEPTH_PULL: f32 = 2e-4;
const TINT: vec4<f32> = vec4<f32>(0.25, 0.61, 1.0, 0.30);

struct VsIn {
    @location(0) pos: vec3<f32>,
}

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    var clip = view.view_proj * vec4<f32>(in.pos, 1.0);
    clip.z = clip.z - DEPTH_PULL * clip.w;
    return clip;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return TINT;
}
