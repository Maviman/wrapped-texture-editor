// UV (2D) view of one texture set (ARCHITECTURE.md 8.4). Fullscreen triangle; `view.view_proj` holds the
// `uv_from_frag` affine that maps a fragment position (physical pixels, origin top-left) to UV. Outside [0,1]^2 a dim
// backdrop; inside the texture over a screen-space checker (straight alpha), linear + mips when minified, nearest above
// 4 pixels per texel and a faint texel grid above 12.

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

struct SetUniforms {
    tex_size: vec4<f32>,
    base_color: vec4<f32>,
    params: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: ViewUniforms;
@group(1) @binding(0) var<uniform> mat_u: SetUniforms;
@group(1) @binding(1) var tex: texture_2d<f32>;
@group(1) @binding(2) var samp_linear: sampler;

const FLAG_ALPHA_CHECKER: u32 = 2u;
const BACKDROP: vec3<f32> = vec3<f32>(0.075, 0.078, 0.086);

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
    let uv = (view.view_proj * vec4<f32>(frag, 0.0, 1.0)).xy;

    // Sampling first: uniform control flow. Nearest is used only when magnified (level 0), read with textureLoad
    // (one texture with two samplers cannot be translated to GLSL).
    let s_linear = textureSample(tex, samp_linear, uv);
    let size = vec2<i32>(textureDimensions(tex, 0));
    let wrapped = uv - floor(uv);
    let texel = clamp(vec2<i32>(floor(wrapped * vec2<f32>(size))), vec2<i32>(0), size - vec2<i32>(1));
    let s_nearest = textureLoad(tex, texel, 0);

    let texels_per_px = max(abs(view.view_proj[0][0]) * mat_u.tex_size.x, 1e-9);
    let px_per_texel = 1.0 / texels_per_px;
    let c = select(s_linear, s_nearest, px_per_texel > 4.0);

    var rgb = c.rgb;
    if (view.params.y & FLAG_ALPHA_CHECKER) != 0u {
        let cell = vec2<i32>(floor(frag / 8.0));
        let odd = ((cell.x + cell.y) & 1) == 1;
        let checker = select(vec3<f32>(0.80), vec3<f32>(0.60), odd);
        rgb = mix(checker, rgb, clamp(c.a, 0.0, 1.0));
    }

    // Faint texel grid when strongly magnified.
    let f = fract(uv * mat_u.tex_size.xy);
    let edge_px = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y)) * px_per_texel;
    let grid_line = clamp(1.0 - edge_px, 0.0, 1.0);
    let strength = smoothstep(12.0, 16.0, px_per_texel) * 0.22;
    let lum = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let grid_color = select(vec3<f32>(0.0), vec3<f32>(1.0), lum < 0.5);
    rgb = mix(rgb, grid_color, grid_line * strength);

    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return vec4<f32>(select(BACKDROP, rgb, inside), 1.0);
}
