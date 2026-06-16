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
    spectrum_lo: vec4<f32>, // FFT bands 0..3: sub, low, low-mid, mid  (MARTIN_FFT-scaled; 0 = off)
    spectrum_hi: vec4<f32>, // FFT bands 4..7: mid-hi, presence, brilliance, air
    warmth: f32,            // harmonic tint: -1 cool (minor/low) .. +1 warm (major/high); 0 = neutral
};
@group(3) @binding(0) var<uniform> bg: BgData;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise (bilinear-interpolated hash) — one octave; fbm stacks several.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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
    } else if (bg.mode == 8u) {
        // FRACTAL — Kaliset orbit-trap (Kali/Mercury): fold p through abs/|z|² with a drifting seed,
        // tracking the closest approach → glowing fractal filaments. No raymarch; iGPU-cheap.
        var z = p * 1.1;
        let c = vec2<f32>(0.9 + 0.08 * sin(t * 0.10), 0.7 + 0.08 * cos(t * 0.13));
        var trap = 1e9;
        for (var i = 0; i < 13; i = i + 1) {
            z = abs(z) / max(dot(z, z), 1e-4) - c;
            trap = min(trap, length(z));
        }
        let g = exp(-6.0 * trap); // bright on the filaments
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.1, 4.2) + trap * 8.0 - t * 0.3)) * g * 0.55;
    } else if (bg.mode == 9u) {
        // CLOUDS — fbm value-noise drifting; a soft volumetric haze. No raymarch; iGPU-cheap.
        var q = p * 1.5 + vec2<f32>(t * 0.05, t * 0.02);
        var f = 0.0;
        var amp = 0.5;
        for (var i = 0; i < 6; i = i + 1) {
            f += amp * vnoise(q);
            q = q * 2.02 + vec2<f32>(1.7, 9.2);
            amp *= 0.5;
        }
        let d = smoothstep(0.15, 0.9, f);
        col = mix(vec3<f32>(0.02, 0.03, 0.06), vec3<f32>(0.45, 0.5, 0.68), d) * 0.6;
    } else {
        // WARP — radial colour swirl (mode 3)
        let r = length(p);
        let a = atan2(p.y, p.x);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.0, 4.0) + a * 3.0 + t - r * 4.0)) * 0.18;
    }

    // beat: the kick brightens the whole field (scaled by MARTIN_BEAT intensity).
    col *= 1.0 + bg.beat.x * 0.6 * bg.beat.w;

    // spectrum (FFT of the rendered track): the bass SWELLS the whole field, the mids wash a slow
    // colour over it, the air SPARKLES — so the backdrop is the music. All zero when MARTIN_FFT=0,
    // making this branch a no-op (backdrop byte-identical to before).
    let bass = bg.spectrum_lo.x + bg.spectrum_lo.y;                       // sub + low
    let mids = bg.spectrum_lo.z + bg.spectrum_lo.w + bg.spectrum_hi.x;    // low-mid..mid
    let air  = bg.spectrum_hi.z + bg.spectrum_hi.w;                       // brilliance + air
    col *= 1.0 + bass * 0.35;
    col += (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.1, 4.2) + bg.time * 0.3)) * mids * 0.05;
    let spk = step(0.992, hash21(floor(uv * vec2<f32>(240.0, 135.0)) + floor(bg.time * 24.0)));
    col += vec3<f32>(0.7, 0.85, 1.0) * spk * air * 0.5;

    // harmonic tint: warm (major/climax) pushes red & drops blue, cool (minor/low) the reverse. 0 = no-op.
    col += bg.warmth * vec3<f32>(0.06, 0.0, -0.06) * (0.4 + length(col));

    col *= bg.dim; // MARTIN_BG_DIM — dial the backdrop down so foreground content reads
    return vec4<f32>(col, 1.0);
}
