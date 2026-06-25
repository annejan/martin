//! Beat-gated **post-processing**: a fullscreen pass over the rendered image that RGB-splits it on
//! the kick (`MARTIN_POST=chroma`) — the whole frame shears red/cyan on every kick, the visceral
//! "the screen is locked to the track" layer. Runs in the camera's `Core3d` render schedule in the
//! `PostProcess` set right after tonemapping, so it covers the window, the offscreen serve target, AND
//! the headless record target uniformly.
//!
//! Deterministic + record-safe: the only input besides the current frame's pixels is `kick`, which is
//! clock-driven (`scene::beat`), so a recording bakes the exact same shear frame-for-frame. No
//! frame-feedback (that would break determinism) — chroma reads only the current frame. Default-off:
//! without `MARTIN_POST` the camera carries no `PostSettings`, so the pass skips that view (no-op).
//!
//! Bevy 0.19: the old `ViewNode` render-graph node became a render-world **system** (`post_pass`) —
//! 0.19's "render graph as systems" overhaul. `RenderContext` is now a system param yielding the
//! command encoder; the view is fetched via the `CurrentView` resource so a view without `PostSettings`
//! is simply skipped. The chroma math + `post.wgsl` are unchanged from the 0.18 node.

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, StoreOp, TextureFormat,
    TextureSampleType,
};
use bevy::render::renderer::{CurrentView, RenderContext, RenderDevice};
use bevy::render::view::ViewTarget;

const POST_SHADER: Handle<Shader> = uuid_handle!("d1f2e3c4-5a6b-7c8d-9e0f-1a2b3c4d5e6f");

pub(crate) const POST_CHROMA: u32 = 1;
pub(crate) const POST_GRAIN: u32 = 2;
pub(crate) const POST_VIGNETTE: u32 = 4;

#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
pub(crate) struct PostSettings {
    pub mode: u32,
    pub intensity: f32, // chroma strength (MARTIN_POST)
    pub kick: f32,      // current beat kick (0..1), set each frame
    pub time: f32,      // clock.t — deterministic film-grain animation
    pub grain: f32,     // film-grain strength
    pub vignette: f32,  // vignette strength
    pub _pad: Vec2,
}

/// Parse `MARTIN_POST` into the camera's settings, or `None` if unset/off. Effects compose via `+`:
/// `chroma` · `grain` · `vignette` · `cine` (= chroma+grain+vignette preset) · `chroma+grain` etc.,
/// each optionally `:strength` (e.g. `cine:1.2`).
pub(crate) fn settings_from_env() -> Option<PostSettings> {
    parse_post(&std::env::var("MARTIN_POST").ok()?)
}

/// Pure parse of a `MARTIN_POST` value into settings (split out so it's unit-testable without env). See
/// [`settings_from_env`] for the format. Returns `None` for an empty/`off`/all-unknown value.
fn parse_post(v: &str) -> Option<PostSettings> {
    let (name, strength) = v
        .split_once(':')
        .map_or((v, 1.0), |(n, s)| (n, s.trim().parse().unwrap_or(1.0)));
    let mut mode = 0u32;
    let (mut grain, mut vignette) = (0.0f32, 0.0f32);
    let mut chroma = 1.0f32;
    for tok in name
        .split(['+', ','])
        .map(|t| t.trim().to_ascii_lowercase())
    {
        match tok.as_str() {
            "chroma" | "rgb" | "rgb-split" | "rgbsplit" | "split" => mode |= POST_CHROMA,
            "grain" | "film" => {
                mode |= POST_GRAIN;
                grain = 0.08;
            }
            "vignette" | "vig" => {
                mode |= POST_VIGNETTE;
                vignette = 0.4;
            }
            "cine" | "cinematic" => {
                mode |= POST_CHROMA | POST_GRAIN | POST_VIGNETTE;
                chroma = 0.7;
                grain = 0.06;
                vignette = 0.32;
            }
            "off" | "" => {}
            other => {
                warn!(
                    "MARTIN_POST: unknown effect '{other}' — try chroma / grain / vignette / cine"
                )
            }
        }
    }
    if mode == 0 {
        return None;
    }
    Some(PostSettings {
        mode,
        intensity: chroma * strength,
        kick: 0.0,
        time: 0.0,
        grain: grain * strength,
        vignette: vignette * strength,
        _pad: Vec2::ZERO,
    })
}

/// Refresh each camera's `kick` from the beat (clock-driven) so the shear lands on the drum. Scaled by
/// the beat *intensity* (the `[sync] beat` / `MARTIN_BEAT` energy curve) so a hushed "breath" section
/// (intensity 0) tears nothing and the drop (high intensity) punches — chroma rides the same curve.
fn drive_post(
    beat: Res<crate::scene::beat::Beat>,
    clock: Res<crate::scene::SeqClock>,
    mut q: Query<&mut PostSettings>,
) {
    for mut s in &mut q {
        s.kick = beat.kick * beat.intensity;
        s.time = clock.t; // deterministic grain animation (record-safe)
    }
}

/// The fullscreen post pass, as a render-world SYSTEM (0.19 render-graph-as-systems). Runs once per
/// rendered view (in the `Core3d` schedule); fetches the current view via `CurrentView` and SKIPS it
/// unless it carries `PostSettings` — so default (no `MARTIN_POST`) is a free no-op.
fn post_pass(
    current: Res<CurrentView>,
    views: Query<(&ViewTarget, &DynamicUniformIndex<PostSettings>)>,
    pipeline_cache: Res<PipelineCache>,
    post_pipeline: Res<PostPipeline>,
    uniforms: Res<ComponentUniforms<PostSettings>>,
    mut ctx: RenderContext,
) {
    let Ok((view_target, settings_index)) = views.get(current.0) else {
        return; // this view has no PostSettings (the common case) → nothing to do
    };
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(post_pipeline.id) else {
        return; // still compiling
    };
    let Some(settings_binding) = uniforms.uniforms().binding() else {
        return;
    };

    // post_process_write() ping-pongs the view target: read source, write destination.
    let post = view_target.post_process_write();
    let layout = pipeline_cache.get_bind_group_layout(&post_pipeline.layout); // descriptor → real layout
    let bind_group = ctx.render_device().create_bind_group(
        "post_bind_group",
        &layout,
        &BindGroupEntries::sequential((
            post.source,
            &post_pipeline.sampler,
            settings_binding.clone(),
        )),
    );

    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("post_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None, // 0.19/wgpu: new field
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[settings_index.index()]);
    pass.draw(0..3, 0..1);
}

#[derive(Resource)]
struct PostPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    id: CachedRenderPipelineId,
}

impl FromWorld for PostPipeline {
    fn from_world(world: &mut World) -> Self {
        // The pipeline holds a layout *descriptor*; the pass resolves it to a real layout via the
        // pipeline cache. So no RenderDevice is needed to build it.
        let layout = BindGroupLayoutDescriptor::new(
            "post_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<PostSettings>(true), // dynamic offset (one per camera)
                ),
            ),
        );
        let sampler = world
            .resource::<RenderDevice>()
            .create_sampler(&SamplerDescriptor::default());
        let vertex = world.resource::<FullscreenShader>().to_vertex_state();
        let id =
            world
                .resource_mut::<PipelineCache>()
                .queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("post_pipeline".into()),
                    layout: vec![layout.clone()],
                    vertex,
                    fragment: Some(FragmentState {
                        shader: POST_SHADER,
                        shader_defs: vec![],
                        entry_point: Some("fragment".into()),
                        // The camera is HDR (camera.rs), so the post-tonemapping target is Rgba16Float —
                        // mirror the tonemapping pipeline's target format exactly.
                        targets: vec![Some(ColorTargetState {
                            format: TextureFormat::Rgba16Float,
                            blend: None,
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    ..default()
                });
        Self {
            layout,
            sampler,
            id,
        }
    }
}

/// Beat-gated fullscreen post-FX. Always registered (cheap); the effect runs only on cameras that
/// carry `PostSettings` (spawned when `MARTIN_POST` is set — see `camera::spawn_camera`).
pub(crate) struct PostPlugin;

impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, POST_SHADER, "../assets/post.wgsl", Shader::from_wgsl);
        app.add_plugins((
            ExtractComponentPlugin::<PostSettings>::default(),
            UniformComponentPlugin::<PostSettings>::default(),
        ))
        .add_systems(Update, drive_post);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        // Run the pass in the per-view Core3d schedule, in the PostProcess set right AFTER tonemapping
        // (where the old graph edge Tonemapping → Post sat) and before upscaling.
        render_app.add_systems(
            Core3d,
            post_pass
                .in_set(Core3dSystems::PostProcess)
                .after(tonemapping),
        );
    }

    // `PostPipeline::from_world` needs RenderDevice + FullscreenShader + PipelineCache, which only
    // exist once the render plugins have finished — so init it here, not in `build`.
    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<PostPipeline>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_post_modes_presets_and_strength() {
        // single effect → its bit, full intensity, no grain/vignette
        let c = parse_post("chroma").unwrap();
        assert_eq!(c.mode, POST_CHROMA);
        assert!((c.intensity - 1.0).abs() < 1e-6 && c.grain == 0.0 && c.vignette == 0.0);
        // aliases fold to the same bit
        assert_eq!(parse_post("rgb").unwrap().mode, POST_CHROMA);
        assert_eq!(parse_post("split").unwrap().mode, POST_CHROMA);
        // composition via + (and case-insensitive)
        assert_eq!(
            parse_post("CHROMA+grain").unwrap().mode,
            POST_CHROMA | POST_GRAIN
        );
        // cine preset = all three bits with its tuned strengths
        let cine = parse_post("cine").unwrap();
        assert_eq!(cine.mode, POST_CHROMA | POST_GRAIN | POST_VIGNETTE);
        assert!((cine.intensity - 0.7).abs() < 1e-6);
        assert!((cine.grain - 0.06).abs() < 1e-6 && (cine.vignette - 0.32).abs() < 1e-6);
        // :strength scales every effect
        let s = parse_post("cine:2").unwrap();
        assert!((s.intensity - 1.4).abs() < 1e-6 && (s.grain - 0.12).abs() < 1e-6);
        // empty / off / all-unknown → None (no post-FX); a bad :strength falls back to 1.0, not a panic
        assert!(parse_post("").is_none());
        assert!(parse_post("off").is_none());
        assert!(parse_post("chorma").is_none()); // typo → unknown → None (warns)
        assert!((parse_post("chroma:abc").unwrap().intensity - 1.0).abs() < 1e-6);
    }
}
