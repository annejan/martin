// martin — fullscreen SHADER INTERLUDE (a `shader:` sequence part), drawn full-frame while the
// splats are cleared. Same effect set as the background (assets/bg.wgsl) but at full brightness and
// with an `alpha` the engine fades in/out across the part. `fx.beat` is x=kick y=snare z=hat
// w=intensity. Edit freely / add a `mode`. Work in `p` (centred, aspect-correct) + time.
#import bevy_pbr::forward_io::VertexOutput

struct FxData {
    time: f32,
    mode: u32,
    aspect: f32,
    alpha: f32,   // fade — driven by the part window (0 at the edges, 1 while held)
    beat: vec4<f32>,
};
@group(3) @binding(0) var<uniform> fx: FxData;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;                                                  // 0..1 across the screen
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(fx.aspect, 1.0) * 2.0; // centred, aspect-correct
    let t = fx.time;
    var col = vec3<f32>(0.0);

    if (fx.mode == 0u) {
        // PLASMA — classic interfering sines
        let v = sin(p.x * 4.0 + t)
              + sin(p.y * 4.0 + t * 1.3)
              + sin((p.x + p.y) * 3.0 + t * 0.7)
              + sin(length(p) * 6.0 - t * 2.0);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.1, 4.2) + v * 1.5 + t * 0.2)) * 0.9;
    } else if (fx.mode == 1u) {
        // TUNNEL — polar warp toward the centre
        let r = length(p) + 1e-3;
        let a = atan2(p.y, p.x);
        let u = a / 6.28318 + t * 0.04;
        let v = 0.5 / r + t * 0.6;
        let c = 0.5 + 0.5 * sin(vec3<f32>(0.0, 2.0, 4.0) + u * 50.24 + v * 3.0);
        col = c * smoothstep(0.0, 0.5, r) * 1.1;
    } else if (fx.mode == 2u) {
        // STARFIELD — twinkling grid, warping toward the centre
        let g = floor(uv * vec2<f32>(90.0, 50.0));
        let h = hash21(g);
        let tw = 0.4 + 0.6 * sin(t * 2.5 + h * 40.0);
        col = vec3<f32>(step(0.972, h) * tw);
    } else {
        // WARP — radial colour swirl
        let r = length(p);
        let a = atan2(p.y, p.x);
        col = (0.5 + 0.5 * cos(vec3<f32>(0.0, 2.0, 4.0) + a * 3.0 + t * 1.5 - r * 4.0)) * 0.8;
    }

    // beat: the kick brightens the whole field (scaled by MARTIN_BEAT intensity).
    col *= 1.0 + fx.beat.x * 0.7 * fx.beat.w;
    // opaque (far plane); fade to BLACK via alpha, so the clearing/returning splats crossfade over it.
    return vec4<f32>(col * fx.alpha, 1.0);
}
