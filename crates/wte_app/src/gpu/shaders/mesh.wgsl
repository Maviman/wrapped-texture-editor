// Mesh shader of the 3D views (ARCHITECTURE.md 8.4). Group 0: view uniforms; group 1: the set (or untextured
// material) uniforms, its texture and the linear sampler; nearest filtering is a textureLoad (one texture with two
// samplers cannot be translated to GLSL). Colours are sRGB-encoded bytes in Rgba8Unorm targets; lighting converts to
// linear and back, unlit output is the texture bytes unchanged.

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
    // x: has texture, y: mip level count of the texture.
    params: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: ViewUniforms;
@group(1) @binding(0) var<uniform> mat_u: SetUniforms;
@group(1) @binding(1) var tex: texture_2d<f32>;
@group(1) @binding(2) var samp_linear: sampler;

const SHADING_CLAY: u32 = 1u;
const SHADING_UV_CHECKER: u32 = 2u;

const FLAG_LIT: u32 = 1u;
const FLAG_ALPHA_CHECKER: u32 = 2u;
const FLAG_NEAREST: u32 = 4u;
const FLAG_MATTE: u32 = 8u;

const CLAY: vec3<f32> = vec3<f32>(0.72, 0.72, 0.72);

// Faces whose normal has a dot product below this with the ring's side normal do not show the ring.
const SIDE_MIN_DOT: f32 = -0.2;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = view.view_proj * vec4<f32>(in.pos, 1.0);
    out.world = in.pos;
    out.normal = in.normal;
    out.uv = in.uv;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x / 12.92;
    let hi = pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, x <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, x <= vec3<f32>(0.0031308));
}

fn hue_rgb(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0)) - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Procedural 8x8 UV checker: hue by column, brightness by row, alternating light/dark cells and a dark marker in the
// top-left corner of every cell, so orientation, mirroring and seams are readable (R2).
fn uv_checker(uv: vec2<f32>) -> vec3<f32> {
    let p = uv * 8.0;
    let cell = vec2<i32>(floor(p));
    let c = ((cell % vec2<i32>(8)) + vec2<i32>(8)) % vec2<i32>(8);
    let f = fract(p);
    let hue = hue_rgb(f32(c.x) / 8.0);
    let shade = 0.5 + 0.5 * (f32(c.y) + 1.0) / 8.0;
    var rgb = mix(vec3<f32>(1.0), hue, 0.8) * shade;
    let odd = ((c.x + c.y) & 1) == 1;
    rgb = select(rgb, mix(rgb, vec3<f32>(1.0), 0.4), odd);
    let marker = f.x < 0.2 && f.y < 0.2;
    return select(rgb, rgb * 0.3, marker);
}

// Transparency checker in texel space (cells of 16 texels after the caller's scaling).
fn alpha_checker(p: vec2<f32>) -> vec3<f32> {
    let cell = vec2<i32>(floor(p));
    let odd = ((cell.x + cell.y) & 1) == 1;
    return select(vec3<f32>(0.82), vec3<f32>(0.62), odd);
}

// Coverage of a ring of half width `half_px` (pixels) centred at signed pixel distance `center` from `s`.
fn ring(s: f32, center: f32, half_px: f32) -> f32 {
    return clamp(half_px + 0.5 - abs(s - center), 0.0, 1.0);
}

// Nearest-neighbour texel with repeat wrapping and the nearest mip level for the UV derivatives (what a nearest
// sampler with a nearest mip filter returns), read with textureLoad.
fn sample_nearest(uv: vec2<f32>, duv_dx: vec2<f32>, duv_dy: vec2<f32>) -> vec4<f32> {
    let levels = i32(max(mat_u.params.y, 1u));
    let size0 = vec2<f32>(textureDimensions(tex, 0));
    let dx = duv_dx * size0;
    let dy = duv_dy * size0;
    let rho2 = max(dot(dx, dx), dot(dy, dy));
    let lod = 0.5 * log2(max(rho2, 1e-12));
    let level = clamp(i32(floor(lod + 0.5)), 0, levels - 1);
    let size = vec2<i32>(textureDimensions(tex, level));
    let wrapped = uv - floor(uv);
    let texel = clamp(vec2<i32>(floor(wrapped * vec2<f32>(size))), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(tex, texel, level);
}

fn safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    return select(fallback, v / max(l, 1e-20), l > 1e-12);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // (1) Everything that needs derivatives runs first, in uniform control flow.
    let s_linear = textureSample(tex, samp_linear, in.uv);
    let s_nearest = sample_nearest(in.uv, dpdx(in.uv), dpdy(in.uv));
    let brush_d = distance(in.world, view.brush.xyz);
    let brush_px = max(fwidth(brush_d), 1e-7);

    let shading = view.params.x;
    let flags = view.params.y;
    let has_texture = mat_u.params.x != 0u;
    let tex_color = select(s_linear, s_nearest, (flags & FLAG_NEAREST) != 0u);

    // (2) Base colour by mode.
    var color = select(mat_u.base_color, tex_color, has_texture);
    if shading == SHADING_CLAY {
        color = vec4<f32>(CLAY, 1.0);
    } else if shading == SHADING_UV_CHECKER {
        color = vec4<f32>(select(CLAY, uv_checker(in.uv), has_texture), 1.0);
    }

    // (3) Transparency.
    var rgb = color.rgb;
    let a = clamp(color.a, 0.0, 1.0);
    if (flags & FLAG_MATTE) != 0u {
        rgb = mix(view.matte.rgb, rgb, a);
    } else if (flags & FLAG_ALPHA_CHECKER) != 0u {
        rgb = mix(alpha_checker(in.uv * mat_u.tex_size.xy / 16.0), rgb, a);
    }

    // (4) Lighting: hemisphere ambient + Lambert key light + faint specular, in linear space.
    if (flags & FLAG_LIT) != 0u {
        let n0 = safe_normalize(in.normal, vec3<f32>(0.0, 1.0, 0.0));
        let n = select(-n0, n0, front);
        let l = safe_normalize(view.light_dir.xyz, vec3<f32>(0.0, 1.0, 0.0));
        let v = safe_normalize(view.eye.xyz - in.world, l);
        let h = safe_normalize(l + v, n);
        let ambient = view.light_dir.w;
        let hemi = mix(vec3<f32>(0.42, 0.40, 0.38), vec3<f32>(1.0, 1.0, 1.02), n.y * 0.5 + 0.5) * ambient;
        let ndl = max(dot(n, l), 0.0);
        let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.06 * select(0.0, 1.0, ndl > 0.0);
        let lin = srgb_to_linear(rgb);
        rgb = linear_to_srgb(lin * (hemi + vec3<f32>(ndl * 0.8)) + vec3<f32>(spec));
    }

    // (5) Brush ring: white/black double ring at the radius, thin inner ring at radius * hardness. With a side normal
    // in brush_aux.yzw (unit; zero = none) the ring is drawn only on faces of that side: the unflipped normal must
    // satisfy dot(n, side) >= SIDE_MIN_DOT, the rule of the Circular brush (core brush.rs SAME_SIDE_MIN_DOT).
    let r = view.brush.w;
    let side = view.brush_aux.yzw;
    let n_side = safe_normalize(in.normal, vec3<f32>(0.0));
    let on_side = dot(side, side) < 0.25 || dot(n_side, side) >= SIDE_MIN_DOT;
    if r > 0.0 && on_side {
        let s = (brush_d - r) / brush_px;
        rgb = mix(rgb, vec3<f32>(0.0), ring(s, 0.75, 0.75));
        rgb = mix(rgb, vec3<f32>(1.0), ring(s, -0.75, 0.75));
        let hardness = clamp(view.brush_aux.x, 0.0, 1.0);
        if hardness > 0.001 && hardness < 0.999 {
            let si = (brush_d - r * hardness) / brush_px;
            rgb = mix(rgb, vec3<f32>(1.0), ring(si, 0.0, 0.5) * 0.7);
        }
    }

    // (6) Opaque output.
    return vec4<f32>(rgb, 1.0);
}
