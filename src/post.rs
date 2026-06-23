//! Beat-gated **post-processing**: a fullscreen pass over the rendered image that RGB-splits it on
//! the kick (`MARTIN_POST=chroma`) — the whole frame shears red/cyan on every kick, the visceral
//! "the screen is locked to the track" layer. Runs in the camera's Core3d graph between tonemapping
//! and the end of post-processing, so it covers the window, the offscreen serve target, AND the
//! headless record target uniformly.
//!
//! Deterministic + record-safe: the only input besides the current frame's pixels is `kick`, which is
//! clock-driven (`scene::beat`), so a recording bakes the exact same shear frame-for-frame. No
//! frame-feedback (that would break determinism) — chroma reads only the current frame. Default-off:
//! without `MARTIN_POST` the camera carries no `PostSettings`, so the node matches no view and is a
//! no-op (existing shows render byte-identically).

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, StoreOp, TextureFormat,
    TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::view::ViewTarget;

const POST_SHADER: Handle<Shader> = uuid_handle!("d1f2e3c4-5a6b-7c8d-9e0f-1a2b3c4d5e6f");

/// Per-camera post-FX settings, also the shader uniform. `mode` 0 = off (pass-through), 1 = chroma.
/// `kick` is refreshed each frame from `Beat` (clock-driven → deterministic). Packed to 16 bytes.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
pub(crate) struct PostSettings {
    pub mode: u32,
    pub intensity: f32, // MARTIN_POST strength
    pub kick: f32,      // current beat kick (0..1), set each frame
    pub _pad: f32,
}

/// Parse `MARTIN_POST` (`chroma` / `chroma:1.5`) into the camera's settings, or `None` if unset/off.
pub(crate) fn settings_from_env() -> Option<PostSettings> {
    let v = crate::env::var("MARTIN_POST").ok()?;
    let (name, strength) = v.split_once(':').map_or((v.as_str(), 1.0), |(n, s)| {
        (n, s.trim().parse().unwrap_or(1.0))
    });
    let mode = match name.trim().to_ascii_lowercase().as_str() {
        "chroma" | "rgb" | "rgb-split" | "rgbsplit" | "split" => 1u32,
        "off" | "" => return None,
        other => {
            warn!("MARTIN_POST: unknown effect '{other}' — try chroma (or chroma:<strength>)");
            return None;
        }
    };
    Some(PostSettings {
        mode,
        intensity: strength,
        kick: 0.0,
        _pad: 0.0,
    })
}

/// Refresh each camera's `kick` from the beat (clock-driven) so the shear lands on the drum. Scaled by
/// the beat *intensity* (the `[sync] beat` / `MARTIN_BEAT` energy curve) so a hushed "breath" section
/// (intensity 0) tears nothing and the drop (high intensity) punches — chroma rides the same curve.
fn drive_post(beat: Res<crate::scene::beat::Beat>, mut q: Query<&mut PostSettings>) {
    for mut s in &mut q {
        s.kick = beat.kick * beat.intensity;
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct PostLabel;

#[derive(Default)]
struct PostNode;

impl ViewNode for PostNode {
    // The node runs only on cameras that carry PostSettings → default-off is free (no match, no-op).
    type ViewQuery = (
        &'static ViewTarget,
        &'static DynamicUniformIndex<PostSettings>,
    );

    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (view_target, settings_index): QueryItem<'w, '_, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<PostPipeline>();
        let cache = world.resource::<PipelineCache>();
        let Some(render_pipeline) = cache.get_render_pipeline(pipeline.id) else {
            return Ok(()); // still compiling
        };
        let uniforms = world.resource::<ComponentUniforms<PostSettings>>();
        let Some(settings_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };

        // post_process_write() ping-pongs the view target: read source, write destination.
        let post = view_target.post_process_write();
        let layout = cache.get_bind_group_layout(&pipeline.layout); // resolve the descriptor → real layout
        let bind_group = render_context.render_device().create_bind_group(
            "post_bind_group",
            &layout,
            &BindGroupEntries::sequential((
                post.source,
                &pipeline.sampler,
                settings_binding.clone(),
            )),
        );

        let mut pass = render_context
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
            });
        pass.set_pipeline(render_pipeline);
        pass.set_bind_group(0, &bind_group, &[settings_index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[derive(Resource)]
struct PostPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    id: CachedRenderPipelineId,
}

impl FromWorld for PostPipeline {
    fn from_world(world: &mut World) -> Self {
        // The pipeline holds a layout *descriptor* (0.18); the node resolves it to a real layout
        // via the pipeline cache. So no RenderDevice is needed to build it.
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
        render_app
            .add_render_graph_node::<ViewNodeRunner<PostNode>>(Core3d, PostLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    PostLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }

    // `PostPipeline::from_world` needs RenderDevice + FullscreenShader + PipelineCache, which only
    // exist once the render plugins have finished — so init it here, not in `build` (the node fetches
    // it at run time, after this).
    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<PostPipeline>();
        }
    }
}
