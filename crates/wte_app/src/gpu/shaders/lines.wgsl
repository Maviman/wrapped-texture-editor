// Instanced wide lines (ARCHITECTURE.md 8.4): the 3D wireframe / seams and the UV wireframe. One instance per edge
// `{ a, b, color }`, 4-vertex triangle strip. The segment is clipped against the near plane, expanded by a fixed
// half width in screen space and pulled towards the camera against z-fighting. For the UV view the instance holds
// `(uv, 0)` and `view_proj` is the UV-to-clip matrix.

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

const HALF_WIDTH_PX: f32 = 1.25;
const DEPTH_PULL: f32 = 2e-4;

struct VsIn {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color: vec4<f32>,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, in: VsIn) -> VsOut {
    var out: VsOut;
    var ca = view.view_proj * vec4<f32>(in.a, 1.0);
    var cb = view.view_proj * vec4<f32>(in.b, 1.0);
    ca.z = ca.z - DEPTH_PULL * ca.w;
    cb.z = cb.z - DEPTH_PULL * cb.w;

    // Near-plane clipping (NDC z in 0..1: the near plane is clip.z == 0).
    if ca.z < 0.0 && cb.z < 0.0 {
        out.pos = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        out.color = vec4<f32>(0.0);
        return out;
    }
    if ca.z < 0.0 {
        ca = mix(ca, cb, ca.z / (ca.z - cb.z));
    }
    if cb.z < 0.0 {
        cb = mix(cb, ca, cb.z / (cb.z - ca.z));
    }

    let half_vp = max(view.viewport.xy * 0.5, vec2<f32>(0.5));
    let sa = ca.xy / ca.w * half_vp;
    let sb = cb.xy / cb.w * half_vp;
    let d = sb - sa;
    let len = length(d);
    let dir = select(vec2<f32>(1.0, 0.0), d / max(len, 1e-20), len > 1e-6);
    let nrm = vec2<f32>(-dir.y, dir.x);

    let at_b = (vi & 2u) != 0u;
    let side = select(-1.0, 1.0, (vi & 1u) != 0u);
    let along = select(-1.0, 1.0, at_b);
    let c = select(ca, cb, at_b);
    let off_ndc = (nrm * side + dir * along) * HALF_WIDTH_PX / half_vp;
    out.pos = vec4<f32>(c.xy + off_ndc * c.w, c.z, c.w);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
