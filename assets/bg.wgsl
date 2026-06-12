// martin — fullscreen BACKGROUND shader (MARTIN_BG=<mode>), drawn behind the splats. Edit freely /
// add your own effect as a new `mode`. Shadertoy-ish: work in `p` (centred, aspect-correct) + time;
// `bg.beat` is x=kick y=snare z=hat w=intensity (beat-reactive). Kept dim — the splats are the star.
#import bevy_pbr::forward_io::VertexOutput

struct BgData {
    time: f32,
    mode: u32,
    aspect: f32,
    dim: f32,
    beat: vec4<f32>,
};
@group(3) @binding(0) var<uniform> bg: BgData;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;                                   // 0..1 across the screen
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(bg.aspect, 1.0) * 2.0; // centred, aspect-correct
    let t = bg.time;
    var col = vec3<f32>(0.0);

    if (bg.mode == 0u) {
        // PLASMA — classic interfering sines
        let v = sin(p.x * 4.0 + t)
              + sin(p.y * 4.0 + t * 1.3)
              + sin((p.x + p.y) * 3.0 + t * 0.7)
              + sin(length(p) * 6.0 - t * 2.0);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.1, 4.2) + v * 1.5 + t * 0.2)) * 0.22;
    } else if (bg.mode == 1u) {
        // TUNNEL — polar warp toward the centre
        let r = length(p) + 1e-3;
        let a = atan2(p.y, p.x);
        let u = a / 6.28318 + t * 0.04;
        let v = 0.5 / r + t * 0.6;
        let c = 0.5 + 0.5 * sin(vec3<f32>(0.0, 2.0, 4.0) + u * 50.24 + v * 3.0);
        col = c * smoothstep(0.0, 0.5, r) * 0.28;
    } else if (bg.mode == 2u) {
        // STARFIELD — small round twinkling points. Each cell of the (square-celled) grid hosts at
        // most one star at a hashed offset with a hashed size, instead of lighting the WHOLE cell
        // (which read as big grey blocks). Cool-white, soft falloff, per-star twinkle phase.
        let cell = uv * vec2<f32>(90.0, 50.0);
        let g = floor(cell);
        let h = hash21(g);
        let f = fract(cell) - 0.5;
        let off = (vec2<f32>(hash21(g + 17.0), hash21(g + 41.0)) - 0.5) * 0.6;
        let d = length(f - off);
        let size = 0.05 + 0.06 * hash21(g + 7.0);
        let tw = 0.55 + 0.45 * sin(t * 2.5 + h * 40.0);
        let star = smoothstep(size, 0.0, d) * step(0.93, h) * tw;
        col = vec3<f32>(0.75, 0.85, 1.0) * star;
    } else if (bg.mode == 4u) {
        // RINGS — concentric pulsing rings rippling out from the centre
        let r = length(p);
        let w = sin(r * 9.0 - t * 3.0);
        let ring = smoothstep(0.6, 1.0, w);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.0, 4.0) + r * 2.0 - t * 0.5)) * ring * 0.3;
    } else if (bg.mode == 5u) {
        // GRID — a neon scrolling grid (flying through a wireframe field)
        let g = abs(fract(p * 4.0 - vec2<f32>(0.0, t * 0.6)) - 0.5);
        let line = smoothstep(0.06, 0.0, min(g.x, g.y));
        col = vec3<f32>(0.1, 0.6, 1.0) * line * 0.35;
    } else if (bg.mode == 6u) {
        // KALEIDO — angular mirror-folded colour wedge spinning slowly
        let r = length(p);
        let a = atan2(p.y, p.x);
        let k = abs(fract(a / 6.28318 * 6.0 + t * 0.08) * 2.0 - 1.0);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.0, 4.0) + k * 5.0 + r * 4.0 - t)) * 0.22;
    } else if (bg.mode == 7u) {
        // BOLT — jagged electric bands flickering across the field
        let v = sin(p.y * 3.0 + t * 5.0 + sin(p.x * 9.0 + t * 2.0) * 2.0);
        col = vec3<f32>(0.5, 0.75, 1.0) * smoothstep(0.92, 1.0, abs(v)) * 0.7;
    } else {
        // WARP — radial colour swirl (mode 3)
        let r = length(p);
        let a = atan2(p.y, p.x);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.0, 4.0) + a * 3.0 + t - r * 4.0)) * 0.18;
    }

    // beat: the kick brightens the whole field (scaled by MARTIN_BEAT intensity).
    col *= 1.0 + bg.beat.x * 0.6 * bg.beat.w;
    col *= bg.dim; // MARTIN_BG_DIM — dial the backdrop down so foreground content reads
    return vec4<f32>(col, 1.0);
}
