// Layer compositing (one pass per visible layer, ping-pong between two targets).
// Must match panoraminal_core::blend::composite and brush::apply_paint.

struct U {
    opacity: f32,
    blend: u32,
    flags: u32,          // 1 = first layer, 2 = live stroke on this layer, 4 = eraser, 8 = alpha lock
    stroke_opacity: f32,
    color: vec4<f32>,
};

@group(0) @binding(0) var prev_tex: texture_2d<f32>;
@group(0) @binding(1) var layer_tex: texture_2d<f32>;
@group(0) @binding(2) var mask_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { return cb * 2.0 * cs; }
    let s = 2.0 * cs - 1.0;
    return cb + s - cb * s;
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb); }
    var d: f32;
    if cb <= 0.25 { d = ((16.0 * cb - 12.0) * cb + 4.0) * cb; } else { d = sqrt(cb); }
    return cb + (2.0 * cs - 1.0) * (d - cb);
}

fn blend_ch(m: u32, cb: f32, cs: f32) -> f32 {
    switch m {
        case 1u: { return cb * cs; }
        case 2u: { return cb + cs - cb * cs; }
        case 3u: { return hard_light(cs, cb); }
        case 4u: { return min(cb, cs); }
        case 5u: { return max(cb, cs); }
        case 6u: { return min(cb + cs, 1.0); }
        case 7u: { return abs(cb - cs); }
        case 8u: { return soft_light(cb, cs); }
        default: { return cs; }
    }
}

fn paint(p: vec4<f32>, a: f32) -> vec4<f32> {
    let eraser = (u.flags & 4u) != 0u;
    let lock = (u.flags & 8u) != 0u;
    if eraser {
        if lock { return p; }
        return vec4<f32>(p.rgb, p.a * (1.0 - a));
    }
    if lock { return vec4<f32>(mix(p.rgb, u.color.rgb, a), p.a); }
    let oa = a + p.a * (1.0 - a);
    if oa <= 0.0 { return p; }
    return vec4<f32>((u.color.rgb * a + p.rgb * p.a * (1.0 - a)) / oa, oa);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let ip = vec2<i32>(pos.xy);
    var s = textureLoad(layer_tex, ip, 0);
    if (u.flags & 2u) != 0u {
        let m = textureLoad(mask_tex, ip, 0).r;
        if m > 0.0 { s = paint(s, min(m * u.stroke_opacity, 1.0)); }
    }
    var b = vec4<f32>(0.0);
    if (u.flags & 1u) == 0u { b = textureLoad(prev_tex, ip, 0); }
    let a_s = s.a * u.opacity;
    if a_s <= 0.0 { return b; }
    let a_b = b.a;
    let a_o = a_s + a_b * (1.0 - a_s);
    var o: vec3<f32>;
    for (var i = 0; i < 3; i++) {
        let cs = s[i];
        let cb = b[i];
        let bl = blend_ch(u.blend, cb, cs);
        o[i] = clamp((a_s * (1.0 - a_b) * cs + a_s * a_b * bl + (1.0 - a_s) * a_b * cb) / a_o, 0.0, 1.0);
    }
    return vec4<f32>(o, clamp(a_o, 0.0, 1.0));
}
