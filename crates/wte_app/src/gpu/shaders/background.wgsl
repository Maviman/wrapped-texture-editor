// Backdrop of the 3D and Preview views (ARCHITECTURE.md 8.4): fullscreen triangle drawn first. `view.params.z` is
// the `Backdrop` (0 dark, 1 light, 2 vertical gradient, 3 checker); `view.viewport` = w, h, 1/w, 1/h in pixels.

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

const BACKDROP_LIGHT: u32 = 1u;
const BACKDROP_GRADIENT: u32 = 2u;
const BACKDROP_CHECKER: u32 = 3u;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - vec2<f32>(1.0), 0.5, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let mode = view.params.z;
    var rgb = vec3<f32>(0.118, 0.122, 0.133);
    if mode == BACKDROP_LIGHT {
        rgb = vec3<f32>(0.80, 0.81, 0.83);
    } else if mode == BACKDROP_GRADIENT {
        let t = clamp(frag.y * view.viewport.w, 0.0, 1.0);
        rgb = mix(vec3<f32>(0.34, 0.36, 0.40), vec3<f32>(0.10, 0.105, 0.115), t);
    } else if mode == BACKDROP_CHECKER {
        let cell = vec2<i32>(floor(frag / 16.0));
        let odd = ((cell.x + cell.y) & 1) == 1;
        rgb = select(vec3<f32>(0.31), vec3<f32>(0.24), odd);
    }
    return vec4<f32>(rgb, 1.0);
}
