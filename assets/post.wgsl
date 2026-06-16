// martin — fullscreen POST-PROCESS (MARTIN_POST). Runs after tonemapping, over the whole rendered
// frame. mode 1 = chroma: an RGB channel split whose magnitude rides the kick → the image shears
// red/cyan on every drum hit. mode 0 = pass-through. Deterministic: `kick` is the clock-driven beat.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;

struct PostSettings {
    mode: u32,
    intensity: f32,
    kick: f32,
    _pad: f32,
};
@group(0) @binding(2) var<uniform> settings: PostSettings;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    if (settings.mode != 1u) {
        return textureSample(screen_texture, screen_sampler, uv); // pass-through
    }
    // CHROMA: shift R outward / B inward along the radial direction from screen centre, magnitude
    // scaling with the kick. At kick=0 the offset is 0 → byte-identical to the source between hits.
    let amt = settings.kick * settings.intensity * 0.012;
    let dir = normalize(uv - vec2<f32>(0.5) + vec2<f32>(1e-5));
    let off = dir * amt;
    let r = textureSample(screen_texture, screen_sampler, uv + off).r;
    let g = textureSample(screen_texture, screen_sampler, uv).g;
    let b = textureSample(screen_texture, screen_sampler, uv - off).b;
    let a = textureSample(screen_texture, screen_sampler, uv).a;
    return vec4<f32>(r, g, b, a);
}
