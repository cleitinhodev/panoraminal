// Layer compositing (one pass per visible layer, ping-pong between two targets).
// Must match panoraminal_core::blend::composite and brush::apply_paint.

struct U {
    opacity: f32,
    blend: u32,
    flags: u32,          // 1 = first layer, 2 = live stroke, 4 = eraser, 8 = alpha lock, 16 = placed image
    stroke_opacity: f32,
    color: vec4<f32>,
    // Floating placed image (see panoraminal_core::place::Frame).
    p_mode: vec4<f32>,   // projection (0 wrap, 1 gnomonic), cos, sin, opacity
    p_f: vec4<f32>,      // forward.xyz, sx
    p_r: vec4<f32>,      // right.xyz, sy
    p_u: vec4<f32>,      // up.xyz, flip bits (1 = x, 2 = y)
    p_c: vec4<f32>,      // plane centre (gnomonic), doc w, doc h
    // Point-based distortion (see panoraminal_core::warp).
    p_h0: vec4<f32>,     // inverse homography row 0, warp flags (1 pin, 2 mesh)
    p_h1: vec4<f32>,
    p_h2: vec4<f32>,
    p_t: vec4<f32>,      // texture w, h, max mip level
    p_mesh: array<vec4<f32>, 8>,  // 4×4 mesh points, two per vec4
};

@group(0) @binding(0) var prev_tex: texture_2d<f32>;
@group(0) @binding(1) var layer_tex: texture_2d<f32>;
@group(0) @binding(2) var mask_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: U;
@group(0) @binding(4) var place_tex: texture_2d<f32>;   // premultiplied RGBA
@group(0) @binding(5) var place_samp: sampler;

const PI: f32 = 3.14159265358979;

// Normalized image coordinates of direction d (|x|,|y| <= 1 inside), or a far
// value when d is behind the projector.
fn place_local(d: vec3<f32>) -> vec2<f32> {
    let f = u.p_f.xyz;
    let r = u.p_r.xyz;
    let up = u.p_u.xyz;
    if u.p_mode.x < 0.5 {
        let x = atan2(dot(d, r), dot(d, f));
        let y = asin(clamp(dot(d, up), -1.0, 1.0));
        return vec2<f32>(x / u.p_f.w, y / u.p_r.w);
    }
    let z = dot(d, f);
    if z <= 1e-6 { return vec2<f32>(9.0, 9.0); }
    let q = vec2<f32>(dot(d, r) / z, dot(d, up) / z) - u.p_c.xy;
    let c = u.p_mode.y;
    let s = u.p_mode.z;
    let l = vec2<f32>(q.x * c + q.y * s, -q.x * s + q.y * c);
    return vec2<f32>(l.x / u.p_f.w, l.y / u.p_r.w);
}

fn mesh_pt(i: u32, j: u32) -> vec2<f32> {
    let k = j * 4u + i;
    let v = u.p_mesh[k / 2u];
    if (k & 1u) == 0u { return v.xy; }
    return v.zw;
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 { return a.x * b.y - a.y * b.x; }

// Inverse bilinear (see warp::inv_bilinear); z = 1 when inside the quad a b c d.
fn inv_bilinear(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>) -> vec3<f32> {
    let e = b - a;
    let f = d - a;
    let g = a - b + c - d;
    let h = p - a;
    let k2 = cross2(g, f);
    let k1 = cross2(e, f) + cross2(h, g);
    let k0 = cross2(h, e);
    var vs = array<f32, 2>(-1.0, -1.0);
    if abs(k2) < 1e-7 * max(abs(k1), 1e-20) {
        vs[0] = -k0 / k1;
    } else {
        let w = k1 * k1 - 4.0 * k0 * k2;
        if w < 0.0 { return vec3<f32>(0.0, 0.0, 0.0); }
        let sw = sqrt(w);
        vs[0] = (-k1 - sw) / (2.0 * k2);
        vs[1] = (-k1 + sw) / (2.0 * k2);
    }
    let eps = 1e-4;
    for (var i = 0; i < 2; i++) {
        let v = vs[i];
        if v >= -eps && v <= 1.0 + eps {
            let den = e + g * v;
            var uu: f32;
            if abs(den.x) >= abs(den.y) { uu = (h.x - f.x * v) / den.x; } else { uu = (h.y - f.y * v) / den.y; }
            if uu >= -eps && uu <= 1.0 + eps {
                return vec3<f32>(clamp(uu, 0.0, 1.0), clamp(v, 0.0, 1.0), 1.0);
            }
        }
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// Frame-local → image coordinates (z = 1 when valid).
fn unwarp(l: vec2<f32>) -> vec3<f32> {
    let flags = u32(u.p_h0.w + 0.5);
    if flags == 0u { return vec3<f32>(l, 1.0); }
    let w = dot(u.p_h2.xyz, vec3<f32>(l, 1.0));
    if abs(w) < 1e-9 { return vec3<f32>(9.0, 9.0, 0.0); }
    let t = vec2<f32>(dot(u.p_h0.xyz, vec3<f32>(l, 1.0)), dot(u.p_h1.xyz, vec3<f32>(l, 1.0))) / w;
    if (flags & 2u) == 0u { return vec3<f32>(t, 1.0); }
    for (var j = 0u; j < 3u; j++) {
        for (var i = 0u; i < 3u; i++) {
            let r = inv_bilinear(t, mesh_pt(i, j), mesh_pt(i + 1u, j), mesh_pt(i + 1u, j + 1u), mesh_pt(i, j + 1u));
            if r.z > 0.5 {
                let g = vec2<f32>(f32(i) + r.x, f32(j) + r.y) / 3.0;
                return vec3<f32>(g.x * 2.0 - 1.0, 1.0 - g.y * 2.0, 1.0);
            }
        }
    }
    return vec3<f32>(9.0, 9.0, 0.0);
}

// Image coordinates of document pixel position px (z = 1 when valid).
fn img_at(px: vec2<f32>) -> vec3<f32> {
    return unwarp(place_local(doc_dir(px)));
}

fn doc_dir(px: vec2<f32>) -> vec3<f32> {
    let lon = px.x / u.p_c.z * 2.0 * PI;
    let lat = (0.5 - px.y / u.p_c.w) * PI;
    let cl = cos(lat);
    return vec3<f32>(cl * cos(lon), sin(lat), cl * sin(lon));
}

// Premultiplied colour of the placed image over one document pixel (2×2 supersampled
// coverage, bilinear texture filtering).
fn placed(px: vec2<f32>) -> vec4<f32> {
    let flips = u32(u.p_u.w + 0.5);
    var acc = vec4<f32>(0.0);
    // Mip level from the pixel footprint (distorted images only; otherwise the
    // texture is a single, pre-downscaled level).
    var lod = 0.0;
    if u.p_t.z > 0.0 {
        let c = img_at(px);
        let a = img_at(px + vec2<f32>(1.0, 0.0));
        let b = img_at(px + vec2<f32>(0.0, 1.0));
        if c.z > 0.5 && a.z > 0.5 && b.z > 0.5 {
            let s = u.p_t.xy * 0.5;
            let jx = length((a.xy - c.xy) * s);
            let jy = length((b.xy - c.xy) * s);
            let major = max(jx, jy);
            let minor = min(jx, jy);
            lod = clamp(log2(max(max(major * 0.5, minor), 1.0)), 0.0, u.p_t.z);
        }
    }
    let offs = array<vec2<f32>, 4>(vec2<f32>(-0.25, -0.25), vec2<f32>(0.25, -0.25),
                                   vec2<f32>(-0.25, 0.25), vec2<f32>(0.25, 0.25));
    for (var k = 0; k < 4; k++) {
        let li = img_at(px + offs[k]);
        var l = li.xy;
        if li.z > 0.5 && abs(l.x) <= 1.0 && abs(l.y) <= 1.0 {
            if (flips & 1u) != 0u { l.x = -l.x; }
            if (flips & 2u) != 0u { l.y = -l.y; }
            let uv = vec2<f32>((l.x + 1.0) * 0.5, (1.0 - l.y) * 0.5);
            acc += textureSampleLevel(place_tex, place_samp, uv, lod) * 0.25;
        }
    }
    return acc * u.p_mode.w;
}

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
    if (u.flags & 16u) != 0u {
        let c = placed(pos.xy);
        if c.a > 1e-5 {
            let rgb = clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
            let a = min(c.a, 1.0);
            if (u.flags & 8u) != 0u {
                s = vec4<f32>(mix(s.rgb, rgb, a), s.a);
            } else {
                let oa = a + s.a * (1.0 - a);
                s = vec4<f32>((rgb * a + s.rgb * s.a * (1.0 - a)) / oa, oa);
            }
        }
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
