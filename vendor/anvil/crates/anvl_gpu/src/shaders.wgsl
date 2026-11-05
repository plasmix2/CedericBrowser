// ANVL_GPU shaders.
//
// Pipelines (all share group 0: globals, the icon atlas, and the sampler):
//   * rounded rectangles: signed distance field with anti-aliased edges and a 1 px border.
//     A rectangle with a softness (params.z) is a drop shadow: its edge fades over that many
//     pixels instead of one. The fill can be a linear or radial gradient.
//   * strokes: line segments with round ends, and arcs, as distance fields.
//   * icons: alpha mask sampled from the icon atlas and tinted.
//   * glyphs: coverage masks from the glyph atlas (one byte per pixel, bound in group 1), each
//     tinted with its own color.
//   * images: full-color textures (photos, video frames) with rounded corners. They use the
//     sampler from group 0 and one texture per draw, bound in group 1.
//   * backdrop blur (ADL-V3 section 3.5): a full-screen copy (fs_blit), Kawase downsample and
//     upsample passes (fs_down, fs_up), and rounded windows onto the blurred copy (fs_blur). The
//     texture being read is bound in group 1, like an image.
//
// Every instance carries a clip rectangle (x0, y0, x1, y1). Pixels whose centers fall outside it
// are not drawn. Output is premultiplied alpha. Coordinates are window pixels, origin top-left.

struct Globals {
    screen: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var icon_tex: texture_2d<f32>;
@group(0) @binding(2) var icon_sampler: sampler;

// The texture for images, glyphs and the blur passes. It is bound per draw, in group 1.
@group(1) @binding(0) var image_tex: texture_2d<f32>;

// Corner of the unit quad for vertex index 0..5 (two triangles).
fn corner(vi: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    return corners[vi];
}

fn to_clip(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        p.x / globals.screen.x * 2.0 - 1.0,
        1.0 - p.y / globals.screen.y * 2.0,
        0.0,
        1.0,
    );
}

// 1 inside the clip rectangle, 0 outside. `p` is a pixel center.
fn clip_cover(p: vec2<f32>, c: vec4<f32>) -> f32 {
    return step(c.x, p.x) * step(p.x, c.z) * step(c.y, p.y) * step(p.y, c.w);
}

// Signed distance to a rounded box: negative inside, positive outside.
fn sd_rounded_box(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p - center) - half_size + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// ---------------------------------------------------------------- rounded rectangles

struct RectIn {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) bounds: vec4<f32>,
    @location(5) fill2: vec4<f32>,
    @location(6) grad: vec4<f32>,
};

struct RectOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) fill: vec4<f32>,
    @location(3) @interpolate(flat) border: vec4<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
    @location(5) @interpolate(flat) bounds: vec4<f32>,
    @location(6) @interpolate(flat) fill2: vec4<f32>,
    @location(7) @interpolate(flat) grad: vec4<f32>,
};

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32, inst: RectIn) -> RectOut {
    // One pixel of padding keeps the anti-aliased edge inside the quad. A shadow's soft edge
    // reaches `softness` past the shape, so its quad grows by that much.
    let pad = 1.0 + inst.params.z;
    let origin = inst.rect.xy - vec2<f32>(pad, pad);
    let size = inst.rect.zw + vec2<f32>(pad * 2.0, pad * 2.0);
    let p = origin + corner(vi) * size;

    var out: RectOut;
    out.pos = to_clip(p);
    out.px = p;
    out.rect = inst.rect;
    out.fill = inst.fill;
    out.border = inst.border;
    out.params = inst.params;
    out.bounds = inst.bounds;
    out.fill2 = inst.fill2;
    out.grad = inst.grad;
    return out;
}

// The fill at `p`: flat, or a ramp from `fill` to `fill2`.
fn fill_at(inp: RectOut, p: vec2<f32>) -> vec4<f32> {
    let kind = inp.grad.x;
    if (kind < 0.5) {
        return inp.fill;
    }
    if (kind > 3.5) {
        // Persona patch: a glowing orb with a wobbling edge and ripples, tinted by `fill2`.
        let res = max(inp.rect.zw, vec2<f32>(1.0, 1.0));
        let rel = (p - inp.rect.xy) / res;
        let c = (rel - vec2<f32>(0.5, 0.5)) * vec2<f32>(res.x / res.y, 1.0);
        let t = inp.grad.y;
        let e = inp.grad.z;
        let r = length(c);
        let ang = atan2(c.y, c.x);
        let wob = 0.022 * sin(ang * 3.0 + t * 1.3) + 0.015 * sin(ang * 5.0 - t * 1.9) + 0.01 * sin(ang * 7.0 + t * 2.7);
        let rad = 0.2 + wob * e;
        let accent = inp.fill2.rgb;
        // A second and third hue, rotated from the accent, so the glow shimmers instead of being flat.
        let alt1 = vec3<f32>(accent.b, accent.r, accent.g);
        let alt2 = vec3<f32>(accent.g, accent.b, accent.r);
        let swirl = 0.5 + 0.5 * sin(ang * 2.0 + t * 0.9);
        let tint = mix(mix(accent, alt1, swirl * 0.5), alt2, (1.0 - swirl) * 0.3);
        let outside = max(r - rad, 0.0);
        let halo = exp(-outside * 6.5) * 0.5;
        let ring_phase = (r - t * 0.06) * 40.0 + sin(ang * 4.0 + t * 1.1) * 1.3;
        let rings = pow(0.5 + 0.5 * sin(ring_phase), 3.0) * exp(-outside * 5.0) * step(rad, r) * 0.5;
        let rim = exp(-abs(r - rad) * 16.0) * 0.55;
        let glow = exp(-pow(r / (rad * 1.05), 2.0) * 1.7);
        var col = tint * (halo + rings + rim) * e;
        col += mix(accent, vec3<f32>(1.0, 1.0, 1.0), 0.4) * glow * (0.92 + 0.08 * sin(t * 2.0));
        // A faint outer haze keeps the area from looking cut off.
        col += accent * exp(-r * 3.2) * 0.12 * e;
        return vec4<f32>(clamp(inp.fill.rgb + col, vec3<f32>(0.0), vec3<f32>(1.0)), inp.fill.a);
    }
    if (kind > 2.5) {
        let res = max(inp.rect.zw, vec2<f32>(1.0, 1.0));
        let rel = clamp((p - inp.rect.xy) / res, vec2<f32>(0.0), vec2<f32>(1.0));
        let uv = vec2<f32>(rel.x, 1.0 - rel.y);
        let aspect = res.x / max(res.y, 1.0);
        let wp = vec2<f32>(uv.x * aspect, uv.y);
        let wt = inp.grad.y * 0.12;

        // Smooth S-curve warp across the field (anvl.dev/hero-shader.js).
        let warp = sin(wp.y * 2.2 - wt * 2.0) * 0.16
                 + sin(wp.x * 1.3 + wt) * 0.1;

        // Diagonal ribbed wave bands.
        let phase = (wp.x * 0.9 + wp.y * 0.45 + warp) * 26.0;
        let height = sin(phase);
        let slope = cos(phase);

        let ridge_step = smoothstep(-0.6, 1.0, height);
        let ridge = ridge_step * ridge_step;
        let shade = slope * 0.5 + 0.5;

        // Cyan on the left, warm gold on the right.
        let warm = smoothstep(0.2, 1.1, wp.x + wp.y * 0.3 - 0.4);
        let cyan = vec3<f32>(0.35, 0.92, 0.98);
        let gold = vec3<f32>(0.99, 0.92, 0.20);
        let tint = mix(cyan, gold, warm);

        var col = tint * ridge * (0.25 + 0.75 * shade);
        let crest_base = max(shade - 0.7, 0.0) * 3.0 * ridge;
        let crest = crest_base * crest_base;
        col += vec3<f32>(0.98, 0.99, 1.0) * crest * 0.6;

        let vignette = smoothstep(0.0, 0.7, uv.x + 0.15) * 0.9;
        col *= vignette * inp.grad.z;

        return vec4<f32>(clamp(inp.fill.rgb + col, vec3<f32>(0.0), vec3<f32>(1.0)), inp.fill.a);
    }
    let half_size = inp.rect.zw * 0.5;
    let center = inp.rect.xy + half_size;
    var t = 0.0;
    if (kind < 1.5) {
        // Linear: 0 at one edge of the box along the direction, 1 at the other.
        let dir = vec2<f32>(cos(inp.grad.y), sin(inp.grad.y));
        let extent = abs(dir.x) * half_size.x + abs(dir.y) * half_size.y;
        t = dot(p - center, dir) / max(extent * 2.0, 1e-3) + 0.5;
    } else {
        // Radial: 0 at the center, 1 at the box's edge.
        t = length((p - center) / max(half_size, vec2<f32>(1e-3, 1e-3)));
    }
    return mix(inp.fill, inp.fill2, clamp(t, 0.0, 1.0));
}

@fragment
fn fs_rect(inp: RectOut) -> @location(0) vec4<f32> {
    let half_size = inp.rect.zw * 0.5;
    let center = inp.rect.xy + half_size;
    let radius = min(inp.params.x, min(half_size.x, half_size.y));
    let d = sd_rounded_box(inp.px, center, half_size, radius);

    // Anti-aliased coverage with a one-pixel feather. A shadow feathers over its softness,
    // centered on the edge, like a CSS box-shadow blur.
    let soft = inp.params.z;
    var cover = clamp(0.5 - d, 0.0, 1.0);
    if (soft > 0.0) {
        cover = 1.0 - smoothstep(-soft * 0.5, soft * 0.5, d);
    }
    cover *= clip_cover(inp.pos.xy, inp.bounds);

    // Border: the inner edge sits `border_width` inside the outer edge.
    let fill = fill_at(inp, inp.px);
    var body = fill;
    let bw = inp.params.y;
    if (bw > 0.0) {
        let inner = clamp(0.5 - (d + bw), 0.0, 1.0);
        body = mix(inp.border, fill, inner);
    }
    let a = body.a * cover;
    return vec4<f32>(body.rgb * a, a);
}

// ---------------------------------------------------------------- strokes

struct PathIn {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) bounds: vec4<f32>,
};

struct PathOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
    @location(1) @interpolate(flat) a: vec4<f32>,
    @location(2) @interpolate(flat) b: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) bounds: vec4<f32>,
};

@vertex
fn vs_path(@builtin(vertex_index) vi: u32, inst: PathIn) -> PathOut {
    // A box around the shape, grown by half the stroke and a pixel for the soft edge.
    let grow = inst.b.x * 0.5 + 1.0;
    var lo: vec2<f32>;
    var hi: vec2<f32>;
    if (inst.b.y < 0.5) {
        lo = min(inst.a.xy, inst.a.zw);
        hi = max(inst.a.xy, inst.a.zw);
    } else {
        lo = inst.a.xy - vec2<f32>(inst.a.z, inst.a.z);
        hi = inst.a.xy + vec2<f32>(inst.a.z, inst.a.z);
    }
    lo -= vec2<f32>(grow, grow);
    hi += vec2<f32>(grow, grow);
    let p = lo + corner(vi) * (hi - lo);
    var out: PathOut;
    out.pos = to_clip(p);
    out.px = p;
    out.a = inst.a;
    out.b = inst.b;
    out.color = inst.color;
    out.bounds = inst.bounds;
    return out;
}

// Distance from `p` to the segment from `s` to `e`.
fn sd_segment(p: vec2<f32>, s: vec2<f32>, e: vec2<f32>) -> f32 {
    let pa = p - s;
    let ba = e - s;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

const TAU: f32 = 6.28318530718;

// Distance from `p` to an arc of radius `r` around `c`, from angle `start` through `sweep`.
fn sd_arc(p: vec2<f32>, c: vec2<f32>, r: f32, start: f32, sweep: f32) -> f32 {
    let q = p - c;
    if (abs(sweep) >= TAU - 1e-4) {
        return abs(length(q) - r);
    }
    // Angle of p measured from the start, in the sweep's direction, in 0..TAU.
    let ang = atan2(q.y, q.x);
    var rel = (ang - start) * sign(sweep);
    rel = rel - floor(rel / TAU) * TAU;
    if (rel <= abs(sweep)) {
        return abs(length(q) - r);
    }
    let e0 = c + r * vec2<f32>(cos(start), sin(start));
    let e1 = c + r * vec2<f32>(cos(start + sweep), sin(start + sweep));
    return min(length(p - e0), length(p - e1));
}

@fragment
fn fs_path(inp: PathOut) -> @location(0) vec4<f32> {
    var d: f32;
    if (inp.b.y < 0.5) {
        d = sd_segment(inp.px, inp.a.xy, inp.a.zw);
    } else {
        d = sd_arc(inp.px, inp.a.xy, inp.a.z, inp.b.z, inp.b.w);
    }
    let cover = clamp(inp.b.x * 0.5 - d + 0.5, 0.0, 1.0) * clip_cover(inp.pos.xy, inp.bounds);
    let a = inp.color.a * cover;
    return vec4<f32>(inp.color.rgb * a, a);
}

// ---------------------------------------------------------------- icons

struct IconIn {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) bounds: vec4<f32>,
};

struct IconOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tint: vec4<f32>,
    @location(2) @interpolate(flat) bounds: vec4<f32>,
};

@vertex
fn vs_icon(@builtin(vertex_index) vi: u32, inst: IconIn) -> IconOut {
    // Rotate the quad around its center. A zero angle leaves it unchanged.
    let c = corner(vi);
    let center = inst.rect.xy + inst.rect.zw * 0.5;
    let local = (c - vec2<f32>(0.5, 0.5)) * inst.rect.zw;
    let a = inst.params.x;
    let cs = cos(a);
    let sn = sin(a);
    let rotated = vec2<f32>(local.x * cs - local.y * sn, local.x * sn + local.y * cs);
    var out: IconOut;
    out.pos = to_clip(center + rotated);
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.tint = inst.tint;
    out.bounds = inst.bounds;
    return out;
}

@fragment
fn fs_icon(inp: IconOut) -> @location(0) vec4<f32> {
    let mask = textureSample(icon_tex, icon_sampler, inp.uv).a * inp.tint.a * clip_cover(inp.pos.xy, inp.bounds);
    return vec4<f32>(inp.tint.rgb * mask, mask);
}

// ---------------------------------------------------------------- glyphs

@vertex
fn vs_glyph(@builtin(vertex_index) vi: u32, inst: IconIn) -> IconOut {
    let c = corner(vi);
    var out: IconOut;
    out.pos = to_clip(inst.rect.xy + c * inst.rect.zw);
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.tint = inst.tint;
    out.bounds = inst.bounds;
    return out;
}

@fragment
fn fs_glyph(inp: IconOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(image_tex, icon_sampler, inp.uv).r * inp.tint.a * clip_cover(inp.pos.xy, inp.bounds);
    return vec4<f32>(inp.tint.rgb * coverage, coverage);
}

// ---------------------------------------------------------------- images

struct ImageOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) tint: vec4<f32>,
    @location(4) @interpolate(flat) radius: f32,
    @location(5) @interpolate(flat) bounds: vec4<f32>,
};

@vertex
fn vs_image(@builtin(vertex_index) vi: u32, inst: IconIn) -> ImageOut {
    let c = corner(vi);
    let p = inst.rect.xy + c * inst.rect.zw;
    var out: ImageOut;
    out.pos = to_clip(p);
    out.px = p;
    out.rect = inst.rect;
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.tint = inst.tint;
    out.radius = inst.params.x;
    out.bounds = inst.bounds;
    return out;
}

@fragment
fn fs_image(inp: ImageOut) -> @location(0) vec4<f32> {
    let half_size = inp.rect.zw * 0.5;
    let center = inp.rect.xy + half_size;
    let radius = min(inp.radius, min(half_size.x, half_size.y));
    let d = sd_rounded_box(inp.px, center, half_size, radius);
    let cover = clamp(0.5 - d, 0.0, 1.0) * clip_cover(inp.pos.xy, inp.bounds);

    // The texture holds straight alpha. Output is premultiplied for the blend state.
    let texel = textureSample(image_tex, icon_sampler, inp.uv);
    let a = texel.a * inp.tint.a * cover;
    return vec4<f32>(texel.rgb * inp.tint.rgb * a, a);
}

// ---------------------------------------------------------------- backdrop blur

struct FullOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle that covers the target. No vertex buffer.
@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FullOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: FullOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_blit(inp: FullOut) -> @location(0) vec4<f32> {
    return textureSample(image_tex, icon_sampler, inp.uv);
}

// Dual Kawase downsample: the center and four diagonal taps, one source texel out.
@fragment
fn fs_down(inp: FullOut) -> @location(0) vec4<f32> {
    let h = 1.0 / vec2<f32>(textureDimensions(image_tex));
    var sum = textureSample(image_tex, icon_sampler, inp.uv) * 4.0;
    sum += textureSample(image_tex, icon_sampler, inp.uv - h);
    sum += textureSample(image_tex, icon_sampler, inp.uv + h);
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(h.x, -h.y));
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(-h.x, h.y));
    return sum / 8.0;
}

// Dual Kawase upsample: eight taps on a diamond, the diagonals weighted twice.
@fragment
fn fs_up(inp: FullOut) -> @location(0) vec4<f32> {
    let h = 1.0 / vec2<f32>(textureDimensions(image_tex));
    var sum = textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(-2.0 * h.x, 0.0));
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(-h.x, h.y)) * 2.0;
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(0.0, 2.0 * h.y));
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(h.x, h.y)) * 2.0;
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(2.0 * h.x, 0.0));
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(h.x, -h.y)) * 2.0;
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(0.0, -2.0 * h.y));
    sum += textureSample(image_tex, icon_sampler, inp.uv + vec2<f32>(-h.x, -h.y)) * 2.0;
    return sum / 12.0;
}

// A rounded window onto the blurred backdrop. The fill alpha says how much of it shows.
@fragment
fn fs_blur(inp: RectOut) -> @location(0) vec4<f32> {
    let backdrop = textureSample(image_tex, icon_sampler, inp.px / globals.screen.xy);
    let half_size = inp.rect.zw * 0.5;
    let center = inp.rect.xy + half_size;
    let radius = min(inp.params.x, min(half_size.x, half_size.y));
    let d = sd_rounded_box(inp.px, center, half_size, radius);
    let a = clamp(0.5 - d, 0.0, 1.0) * inp.fill.a * clip_cover(inp.pos.xy, inp.bounds);
    return vec4<f32>(backdrop.rgb * a, a);
}
