// martin — fullscreen POST-PROCESS (MARTIN_POST). Runs after tonemapping, over the whole rendered
// frame. `mode` is a BITFIELD: bit0 = chroma (RGB shear on the kick), bit1 = film grain, bit2 =
// vignette. `cine` sets all three. Deterministic/record-safe: chroma rides the clock-driven `kick`;
// grain is a pure hash of pixel + the clock-driven `time` (no wall-clock, no frame feedback).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;

struct PostSettings {
    mode: u32,
    intensity: f32,
    kick: f32,
    time: f32,
    grain: f32,
    vignette: f32,
    _pad: vec2<f32>,
};
@group(0) @binding(2) var<uniform> settings: PostSettings;

// cheap hash → [0,1), pure fn of its input (Dave Hoskins' hash12).
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    if (settings.mode == 0u) {
        return textureSample(screen_texture, screen_sampler, uv); // pass-through
    }

    // CHROMA (bit0): shift R outward / B inward radially from centre, magnitude rides the kick. At
    // kick=0 the offset is 0 → the three taps collapse to one (byte-identical between hits).
    let amt = select(0.0, settings.kick * settings.intensity * 0.012, (settings.mode & 1u) != 0u);
    var rgb: vec3<f32>;
    var a: f32;
    if (amt != 0.0) {
        let dir = normalize(uv - vec2<f32>(0.5) + vec2<f32>(1e-5));
        let off = dir * amt;
        rgb = vec3<f32>(
            textureSample(screen_texture, screen_sampler, uv + off).r,
            textureSample(screen_texture, screen_sampler, uv).g,
            textureSample(screen_texture, screen_sampler, uv - off).b,
        );
        a = textureSample(screen_texture, screen_sampler, uv).a;
    } else {
        let c = textureSample(screen_texture, screen_sampler, uv);
        rgb = c.rgb;
        a = c.a;
    }

    // VIGNETTE (bit2): darken toward the corners — a soft cinematic frame.
    if ((settings.mode & 4u) != 0u) {
        let d = distance(uv, vec2<f32>(0.5)) * 1.41421356;
        rgb *= 1.0 - settings.vignette * smoothstep(0.45, 1.0, d);
    }

    // GRAIN (bit1): per-pixel, per-frame deterministic noise (hash of uv + the clock-driven time).
    if ((settings.mode & 2u) != 0u) {
        let n = hash12(uv * 1024.0 + vec2<f32>(settings.time * 60.0, settings.time * 37.0)) - 0.5;
        rgb += vec3<f32>(n) * settings.grain;
    }

    return vec4<f32>(rgb, a);
}
