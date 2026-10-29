// Displays the composited equirectangular image either as a 360° view (exact
// per-pixel ray → longitude/latitude, no sphere mesh) or as a flat 2D map.

struct D {
    fwd: vec4<f32>,      // xyz forward, w = tan(fov_y/2)
    right: vec4<f32>,    // xyz right,   w = tan(fov_x/2)
    up: vec4<f32>,       // xyz up,      w = mode (0 sphere, 1 flat)
    vp: vec4<f32>,       // viewport x, y, w, h (physical pixels)
    flat: vec4<f32>,     // cx, cy, zoom, checker cell size (px)
    check_a: vec4<f32>,  // rgb, w = grid on (0/1)
    check_b: vec4<f32>,  // rgb, w = srgb output (0/1)
    bg: vec4<f32>,       // outside-image colour, w = wrapped copy dimming
    grid: vec4<f32>,     // grid rgb, w = grid alpha
};

@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> d: D;

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn grid_line(lon: f32, lat: f32, px_ang: f32) -> f32 {
    let step = PI / 12.0; // 15°
    let dlat = abs(lat - round(lat / step) * step);
    let dlon = abs(lon - round(lon / step) * step) * max(cos(lat), 0.0);
    let w = px_ang * 0.75;
    var g = 1.0 - smoothstep(w, w * 2.0, min(dlat, dlon));
    // Equator and the 0°/180° meridian are emphasised.
    let eq = 1.0 - smoothstep(w * 1.5, w * 3.0, abs(lat));
    g = max(g * 0.55, eq);
    return g;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let local = pos.xy - d.vp.xy;
    var uv: vec2<f32>;
    var lon: f32;
    var lat: f32;
    var px_ang: f32;
    var dim = 1.0;
    var outside = false;
    if d.up.w < 0.5 {
        let nx = local.x / d.vp.z * 2.0 - 1.0;
        let ny = -(local.y / d.vp.w * 2.0 - 1.0);
        let dir = normalize(d.fwd.xyz + d.right.xyz * (nx * d.right.w) + d.up.xyz * (ny * d.fwd.w));
        lon = atan2(dir.z, dir.x);
        lat = asin(clamp(dir.y, -1.0, 1.0));
        uv = vec2<f32>(fract(lon / TAU), 0.5 - lat / PI);
        px_ang = 2.0 * d.fwd.w / d.vp.w;
    } else {
        let iw = d.vp.z * d.flat.z;
        let u = d.flat.x + (local.x - d.vp.z * 0.5) / iw;
        let v = d.flat.y + (local.y - d.vp.w * 0.5) / (iw * 0.5);
        if v < 0.0 || v > 1.0 { outside = true; }
        if u < 0.0 || u >= 1.0 { dim = d.bg.w; }
        uv = vec2<f32>(fract(u), v);
        lon = uv.x * TAU;
        if lon > PI { lon = lon - TAU; }
        lat = (0.5 - v) * PI;
        px_ang = TAU / iw;
    }
    var col: vec3<f32>;
    if outside {
        col = d.bg.rgb;
    } else {
        let c = textureSampleLevel(img, samp, uv, 0.0);
        let cell = max(d.flat.w, 1.0);
        let k = (i32(floor(pos.x / cell)) + i32(floor(pos.y / cell))) & 1;
        let chk = select(d.check_a.rgb, d.check_b.rgb, k == 1);
        col = mix(chk, c.rgb, c.a);
        if d.check_a.w > 0.5 {
            col = mix(col, d.grid.rgb, grid_line(lon, lat, px_ang) * d.grid.w);
        }
        col = mix(d.bg.rgb, col, dim);
    }
    if d.check_b.w > 0.5 { col = to_linear(col); }
    return vec4<f32>(col, 1.0);
}
